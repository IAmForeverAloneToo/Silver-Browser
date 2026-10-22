//! A controlled document's requests through its service worker (M39 step
//! 5): `FetchEvent`, `respondWith` and what it may answer, the fallback
//! to the network, as the reference browser answered probe p2's page
//! (scratchpad `m39/pages/p2`, Chrome 155), its navigations aside.

use net::test_server::{Reply, TestServer};

use crate::testing::*;

fn js(body: &str) -> Reply {
    Reply::with_fields(
        200,
        &[
            ("Content-Type", "text/javascript"),
            ("Cache-Control", "no-store"),
        ],
        body.as_bytes(),
    )
}

/// Probe p2's worker, its fetches of another origin's at `elsewhere`.
fn worker(elsewhere: &str) -> String {
    format!(
        "const seen = [];
self.addEventListener('install', e => e.waitUntil(caches.open('c').then(c => c.put('cached', new Response('from the cache', {{headers: {{'x-sw': 'cache'}}}})))));
self.addEventListener('activate', e => e.waitUntil(clients.claim()));
self.addEventListener('fetch', e => {{
  // The engine has no URL class: the path and the query cut by hand.
  const rest = '/' + e.request.url.split('/').slice(3).join('/');
  const at = rest.indexOf('?');
  const u = {{ search: at < 0 ? '' : rest.slice(at) }};
  const p = (at < 0 ? rest : rest.slice(0, at)).replace('/p2/', '');
  seen.push(p + ':' + e.request.mode + ':' + e.request.destination + ':' + e.request.method + ':' + (e.clientId ? 'c' : '-') + (e.resultingClientId ? 'r' : '-') + ':' + Object.getPrototypeOf(e).constructor.name);
  if (p.startsWith('synth')) e.respondWith(new Response('made ' + e.request.method + ' ' + u.search, {{status: 201, statusText: 'Made', headers: {{'x-sw': '1', 'content-type': 'text/x-made'}}}}));
  else if (p === 'missing.txt') e.respondWith(fetch(e.request));
  else if (p === 'error') e.respondWith(Response.error());
  else if (p === 'reject') e.respondWith(Promise.reject(new Error('no')));
  else if (p === 'twice') {{ e.respondWith(new Response('one')); try {{ e.respondWith(new Response('two')); }} catch (x) {{ seen.push('twice ' + x.name + ': ' + x.message); }} }}
  else if (p === 'late') {{ setTimeout(() => {{ try {{ e.respondWith(new Response('late')); }} catch (x) {{ seen.push('late ' + x.name + ': ' + x.message); }} }}, 0); }}
  else if (p === 'redirect') e.respondWith(Response.redirect('real.txt', 302));
  else if (p === 'opaque') e.respondWith(fetch('{elsewhere}/p2/real.txt', {{mode: 'no-cors'}}));
  else if (p === 'cached') e.respondWith(caches.match(e.request));
}});
self.addEventListener('message', e => e.source.postMessage(seen.join(' ')));"
    )
}

/// Probe p2's page, its frame aside, its log the console's.
fn page(elsewhere: &str) -> String {
    format!(
        "<!doctype html><script>
const log = (s) => console.log(s);
const round = Number(sessionStorage.getItem('round') || '0');
sessionStorage.setItem('round', round + 1);
const show = async (name, p) => {{
  try {{
    const r = await p;
    const text = await r.text();
    log(name + ': ' + r.status + ' ' + JSON.stringify(r.statusText) + ' type ' + r.type + ' url ' + r.url.replace(location.origin, '') + ' redirected ' + r.redirected + ' ct ' + r.headers.get('content-type') + ' x ' + r.headers.get('x-sw') + ' body ' + JSON.stringify(text.slice(0, 40)));
  }} catch (e) {{ log(name + ': ' + e.name + ': ' + e.message); }}
}};
(async () => {{
  if (round === 0) {{
    await navigator.serviceWorker.register('sw.js');
    await navigator.serviceWorker.ready;
    location.reload();
    return;
  }}
  log('controlled ' + !!navigator.serviceWorker.controller);
  await show('synth', fetch('synth?q=1'));
  await show('pass', fetch('real.txt'));
  await show('missing', fetch('missing.txt'));
  await show('error', fetch('error'));
  await show('reject', fetch('reject'));
  await show('twice', fetch('twice'));
  await show('late', fetch('late'));
  await show('redirect', fetch('redirect'));
  await show('cross', fetch('{elsewhere}/p2/real.txt', {{mode: 'no-cors'}}));
  await show('opaque-to-cors', fetch('opaque'));
  await show('post', fetch('synth', {{method: 'POST', body: 'b'}}));
  await show('cached', fetch('cached'));
  const img = new Image(); img.src = 'img.png';
  await new Promise(r => {{ img.onload = img.onerror = r; }});
  const seen = await new Promise(r => {{ navigator.serviceWorker.onmessage = e => r(e.data); navigator.serviceWorker.controller.postMessage('seen?'); }});
  log('sw saw ' + seen);
  log('done');
}})().catch(e => {{ log('error ' + e.name + ': ' + e.message); log('done'); }});
</script>"
    )
}

#[test]
fn a_controlled_documents_requests_go_through_its_worker_as_the_reference_browsers_do() {
    let other =
        serve(|path| (path == "/p2/real.txt").then(|| Reply::ok("text/plain", b"the real text")));
    let elsewhere = other.url("/").origin().serialization();
    let (sw, body) = (worker(&elsewhere), page(&elsewhere));
    let server: TestServer = serve(move |path| match path {
        "/p2/index.html" => Some(html(&body)),
        "/p2/sw.js" => Some(js(&sw)),
        "/p2/real.txt" => Some(Reply::ok("text/plain", b"the real text")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/p2/index.html"));
    let lines = tab.run_until(|lines| lines.iter().any(|l| l == "done"));
    let origin = server.url("/").origin().serialization();
    // The console's own lines of the fetches that failed aside.
    let lines: Vec<String> = lines
        .into_iter()
        .filter(|l| !l.starts_with("Failed to fetch ") && !l.starts_with("Failed to load resource"))
        .map(|l| {
            l.replace(&origin, "ORIGIN")
                .replace(&elsewhere, "ELSEWHERE")
        })
        .collect();
    assert_eq!(
        lines,
        [
            "controlled true",
            "synth: 201 \"Made\" type basic url /p2/synth?q=1 redirected false ct text/x-made x 1 body \"made GET ?q=1\"",
            "pass: 200 \"Status\" type basic url /p2/real.txt redirected false ct text/plain x null body \"the real text\"",
            // The test server's reason phrase is "Status", where the
            // reference's server said "OK" and "Not Found".
            "missing: 404 \"Status\" type basic url /p2/missing.txt redirected false ct text/plain x null body \"not here\"",
            "error: TypeError: Failed to fetch",
            "reject: TypeError: Failed to fetch",
            "twice: 200 \"\" type basic url /p2/twice redirected false ct text/plain;charset=UTF-8 x null body \"one\"",
            "late: 404 \"Status\" type basic url /p2/late redirected false ct text/plain x null body \"not here\"",
            "redirect: 200 \"Status\" type basic url /p2/real.txt redirected true ct text/plain x null body \"the real text\"",
            "cross: 0 \"\" type opaque url  redirected false ct null x null body \"\"",
            "opaque-to-cors: TypeError: Failed to fetch",
            "post: 201 \"Made\" type basic url /p2/synth redirected false ct text/x-made x 1 body \"made POST \"",
            "cached: 200 \"\" type basic url /p2/cached redirected false ct text/plain;charset=UTF-8 x cache body \"from the cache\"",
            "sw saw synth:cors::GET:c-:FetchEvent real.txt:cors::GET:c-:FetchEvent missing.txt:cors::GET:c-:FetchEvent error:cors::GET:c-:FetchEvent reject:cors::GET:c-:FetchEvent twice:cors::GET:c-:FetchEvent twice InvalidStateError: Failed to execute 'respondWith' on 'FetchEvent': respondWith() was already called. late:cors::GET:c-:FetchEvent late InvalidStateError: Failed to execute 'respondWith' on 'FetchEvent': The event handler is already finished. redirect:cors::GET:c-:FetchEvent real.txt:cors::GET:c-:FetchEvent real.txt:no-cors::GET:c-:FetchEvent opaque:cors::GET:c-:FetchEvent synth:cors::POST:c-:FetchEvent cached:cors::GET:c-:FetchEvent img.png:no-cors:image:GET:c-:FetchEvent",
            "done",
        ]
    );
}
