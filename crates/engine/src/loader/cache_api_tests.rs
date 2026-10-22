//! The Cache API in pages and workers (M39): what a script sees, as the
//! reference browser answered the same calls (scratchpad
//! `m39/pages/p3`, Chrome 155), and what is kept apart and kept.

use net::test_server::{Reply, TestServer};

use crate::testing::*;

/// A page whose script runs `body` (an async function's) and logs
/// "done" after it.
fn page(body: &str) -> String {
    format!(
        "<script>\
         const tryit = async (name, f) => {{ try {{ const v = await f(); console.log(name + ': ' + v); }} \
         catch (e) {{ console.log(name + ': ' + e.name + ': ' + e.message); }} }};\
         const desc = (r) => r ? (r.status + ' ' + r.url.replace(location.origin, '') + ' ' + r.type + ' ' + r.headers.get('x-n')) : String(r);\
         (async () => {{ {body} }})().then(() => console.log('done'), (e) => console.log('failed ' + e));\
         </script>"
    )
}

fn server(body: String) -> TestServer {
    serve(move |path| match path {
        "/page.html" => Some(html(&body)),
        "/real.txt" => Some(Reply::ok("text/plain", b"the real text")),
        _ => None,
    })
}

fn run(server: &TestServer) -> Vec<String> {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    let lines = tab.run_until(|lines| lines.iter().any(|l| l == "done" || l.starts_with("failed")));
    let origin = server.url("/").origin().serialization();
    lines
        .into_iter()
        .filter(|l| l != "done")
        .map(|l| l.replace(&origin, "ORIGIN"))
        .collect()
}

#[test]
fn a_pages_caches_answer_as_the_reference_browsers_do() {
    let other = serve(|_| Some(Reply::ok("text/plain", b"elsewhere")));
    let elsewhere = other.url("/real.txt").to_string();
    let body = page(&format!(
        "\
        await tryit('protos', async () => [Object.prototype.toString.call(caches), typeof caches.open, typeof caches.match].join(' '));\
        const a = await caches.open('a');\
        await tryit('open again same', async () => (await caches.open('a')) === a);\
        await tryit('keys order', async () => {{ await caches.open('z'); await caches.open('b'); return (await caches.keys()).join(','); }});\
        await tryit('has', async () => [await caches.has('a'), await caches.has('nope')].join(','));\
        const resp = (n, h = {{}}) => new Response('body ' + n, {{headers: Object.assign({{'x-n': String(n)}}, h)}});\
        await a.put('one', resp(1));\
        await a.put(new Request('two?x=1'), resp(2));\
        await a.put('three', resp(3, {{vary: 'x-v'}}));\
        await tryit('put returns', async () => String(await a.put('four', resp(4))));\
        await tryit('match one', async () => desc(await a.match('one')));\
        await tryit('match text', async () => (await a.match('one')).text());\
        await tryit('match two ignoreSearch', async () => desc(await a.match('two', {{ignoreSearch: true}})) + ' / plain ' + desc(await a.match('two')));\
        await tryit('match POST', async () => desc(await a.match(new Request('one', {{method: 'POST'}}))) + ' / ignoreMethod ' + desc(await a.match(new Request('one', {{method: 'POST'}}), {{ignoreMethod: true}})));\
        await tryit('vary', async () => desc(await a.match(new Request('three', {{headers: {{'x-v': 'y'}}}}))) + ' / ignoreVary ' + desc(await a.match(new Request('three', {{headers: {{'x-v': 'y'}}}}), {{ignoreVary: true}})));\
        await tryit('keys', async () => (await a.keys()).map(r => r.url.replace(location.origin, '') + ':' + r.method).join(','));\
        await tryit('matchAll', async () => (await a.matchAll()).map(desc).join(','));\
        await a.put('one', resp(11));\
        await tryit('replaced keeps place', async () => (await a.keys()).map(r => r.url.replace(location.origin + '/', '')).join(','));\
        await tryit('caches.match', async () => {{ const b = await caches.open('b'); await b.put('one', resp(99)); return desc(await caches.match('one')) + ' / named ' + desc(await caches.match('one', {{cacheName: 'b'}})); }});\
        await tryit('delete', async () => [await a.delete('one'), await a.delete('one'), (await a.keys()).length].join(','));\
        await tryit('put POST', () => a.put(new Request('p', {{method: 'POST'}}), resp(5)));\
        await tryit('put used body', async () => {{ const r = resp(6); await r.text(); return a.put('used', r); }});\
        await tryit('put 206', () => a.put('partial', new Response('x', {{status: 206}})));\
        await tryit('put vary star', () => a.put('star', resp(7, {{vary: '*'}})));\
        await tryit('put data url', () => a.put('data:text/plain,x', resp(8)));\
        await tryit('put 404 ok', async () => {{ await a.put('nf', new Response('x', {{status: 404}})); return desc(await a.match('nf')); }});\
        await tryit('add ok', async () => {{ await a.add('real.txt'); return desc(await a.match('real.txt')); }});\
        await tryit('add 404', () => a.add('missing.txt'));\
        await tryit('addAll dup', () => a.addAll(['real.txt', 'real.txt']));\
        await tryit('add opaque', async () => {{ await a.add(new Request('{elsewhere}', {{mode: 'no-cors'}})); return 'added'; }});\
        await tryit('put opaque', async () => {{ const r = await fetch('{elsewhere}', {{mode: 'no-cors'}}); await a.put('op', r); return desc(await a.match('op')); }});\
        await tryit('cross origin key', async () => {{ await a.put('{elsewhere}x', resp(9)); return desc(await a.match('{elsewhere}x')); }});\
        await tryit('fragment', async () => {{ await a.put('frag#x', resp(10)); return desc(await a.match('frag#y')) + ' ' + (await a.keys()).map(r => r.url).filter(u => u.includes('frag')).map(u => u.replace(location.origin, '')).join(','); }});\
        await tryit('match body used', async () => {{ const r = await a.match('four'); const r2 = await a.match('four'); return [r === r2, r.bodyUsed].join(','); }});\
        await tryit('estimate', async () => {{ const e = await navigator.storage.estimate(); return [e.usage > 7 * 1024 * 1024, e.usageDetails.caches === e.usage].join(); }});\
        await tryit('caches.delete open', async () => {{ await caches.delete('a'); return desc(await a.match('four')) + ' has ' + await caches.has('a'); }});\
        "
    ));
    let server = server(body);
    let lines = run(&server);
    let wanted = [
        "protos: [object CacheStorage] function function",
        "open again same: false",
        "keys order: a,z,b",
        "has: true,false",
        "put returns: undefined",
        "match one: 200  default 1",
        "match text: body 1",
        "match two ignoreSearch: 200  default 2 / plain undefined",
        "match POST: undefined / ignoreMethod 200  default 1",
        "vary: undefined / ignoreVary 200  default 3",
        "keys: /one:GET,/two?x=1:GET,/three:GET,/four:GET",
        "matchAll: 200  default 1,200  default 2,200  default 3,200  default 4",
        "replaced keeps place: two?x=1,three,four,one",
        "caches.match: 200  default 11 / named 200  default 99",
        "delete: true,false,3",
        "put POST: TypeError: Failed to execute 'put' on 'Cache': Request method 'POST' is unsupported",
        "put used body: TypeError: Failed to execute 'put' on 'Cache': Response body is already used",
        "put 206: TypeError: Failed to execute 'put' on 'Cache': Partial response (status code 206) is unsupported",
        "put vary star: TypeError: Failed to execute 'put' on 'Cache': Vary header contains *",
        "put data url: TypeError: Failed to execute 'put' on 'Cache': Request scheme 'data' is unsupported",
        "put 404 ok: 404  default null",
        "add ok: 200 /real.txt basic null",
        "add 404: TypeError: Failed to execute 'add' on 'Cache': Request failed",
        "addAll dup: InvalidStateError: Failed to execute 'addAll' on 'Cache': Cache.addAll(): duplicate requests (ORIGIN/real.txt)",
        "add opaque: TypeError: Failed to execute 'add' on 'Cache': Request failed",
        "put opaque: 0  opaque null",
        "cross origin key: 200  default 9",
        "fragment: 200  default 10 /frag#x",
        "match body used: false,false",
        "estimate: true,true",
        "caches.delete open: 200  default 4 has false",
    ];
    assert_eq!(lines, wanted);
}

#[test]
fn caches_are_kept_by_storage_key_and_in_the_profile() {
    let write =
        page("const c = await caches.open('kept'); await c.put('k', new Response('kept body'));");
    let read = page(
        "console.log('names ' + (await caches.keys()).join(','));          const r = await caches.match('k'); console.log('body ' + (r && await r.text()));",
    );
    let server = serve(move |path| match path {
        "/write.html" => Some(html(&write)),
        "/read.html" => Some(html(&read)),
        _ => None,
    });
    let other = {
        let read = page(
            "console.log('names ' + (await caches.keys()).join(','));              const r = await caches.match('k'); console.log('body ' + (r && await r.text()));",
        );
        serve(move |path| (path == "/read.html").then(|| html(&read)))
    };
    let memory = profile::Memory::new();
    let tab_in = |context: &crate::Context| {
        let mut tab = Tab::new();
        tab.engine.set_scripting(true);
        tab.engine.join(context);
        tab
    };
    let lines_of = |tab: &mut Tab, url| {
        tab.engine.navigate(url);
        let lines = tab.run_until(|lines| lines.iter().any(|l| l == "done"));
        lines
            .into_iter()
            .filter(|l| l != "done")
            .collect::<Vec<_>>()
    };
    {
        let context = crate::Context::new();
        context
            .set_profile(profile::Profile::in_memory(&memory).unwrap())
            .unwrap();
        let mut tab = tab_in(&context);
        lines_of(&mut tab, server.url("/write.html"));
        // Another origin has none of them.
        let mut stranger = tab_in(&context);
        assert_eq!(
            lines_of(&mut stranger, other.url("/read.html")),
            ["names ", "body undefined"]
        );
        context
            .flush_profile(std::time::Duration::from_secs(10))
            .unwrap();
    }
    // The profile opened again: its caches, their bodies read from it.
    let context = crate::Context::new();
    context
        .set_profile(profile::Profile::in_memory(&memory).unwrap())
        .unwrap();
    let mut tab = tab_in(&context);
    assert_eq!(
        lines_of(&mut tab, server.url("/read.html")),
        ["names kept", "body kept body"]
    );
}

#[test]
fn a_worker_reaches_its_creators_caches() {
    let body = page(
        "await (await caches.open('w')).put('k', new Response('from the page'));         const w = new Worker('w.js');         console.log(await new Promise(r => w.onmessage = e => r(e.data)));",
    );
    let server = serve(move |path| {
        match path {
        "/page.html" => Some(html(&body)),
        "/w.js" => Some(Reply::ok(
            "text/javascript",
            b"(async () => { const r = await caches.match('k');               postMessage([typeof CacheStorage, await r.text(), (await caches.keys()).join()].join(' ')); })();",
        )),
        _ => None,
    }
    });
    assert_eq!(run(&server), ["function from the page w"]);
}

#[test]
fn an_insecure_or_sandboxed_context_has_no_caches() {
    let body = page(
        "const frame = (attrs, src) => new Promise(r => { const f = document.createElement('iframe');            for (const [k, v] of Object.entries(attrs)) f.setAttribute(k, v); f.srcdoc = src;            onmessage = e => r(e.data); document.documentElement.append(f); });         console.log('sandboxed ' + await frame({sandbox: 'allow-scripts'},            '<script>let s; try { caches; s = \"no throw\"; } catch (e) { s = e.name + \": \" + e.message; } parent.postMessage(s, \"*\")<\\/script>'));",
    );
    let server = server(body);
    assert_eq!(
        run(&server),
        [
            "sandboxed SecurityError: Failed to read the 'caches' property from 'Window': Cache storage is disabled because the context is sandboxed and lacks the 'allow-same-origin' flag."
        ]
    );
}

#[test]
fn an_insecure_context_has_no_cache_api_at_all() {
    // M39: `[SecureContext]`, as the reference browser has it: not a
    // property that throws, but none.
    let server = serve(|path| {
        (path == "/page.html").then(|| {
            html(
                "<script>console.log([isSecureContext, typeof caches, 'caches' in self, \
                 typeof CacheStorage, typeof Cache, typeof Request].join(' '));</script>",
            )
        })
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.set_network(net::Client::new(
        net::transport::Config::default()
            .resolve("insecure.test", std::net::IpAddr::from([127, 0, 0, 1])),
    ));
    let url =
        net::Url::parse(&format!("http://insecure.test:{}/page.html", server.port())).unwrap();
    tab.engine.navigate(url);
    let lines = tab.run_until(|lines| !lines.is_empty());
    assert_eq!(
        lines,
        ["false undefined false undefined undefined function"]
    );
}
