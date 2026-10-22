//! Service workers' clients and messages in one process (M39 step 4):
//! a document becomes a client, controlled once a registration of its
//! URL has an active version; messages cross both ways; `clients`
//! lists, finds and claims; a new version activated or claiming
//! becomes the controller, `controllerchange` heard. As the reference
//! browser answered the same pages (scratchpad `m39/pages/p1`, `p5`,
//! `p6`; Chrome 155), lines whose order is the threads' compared apart.

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

/// The page's lines until "done", across its reloads, its origin named.
fn run(server: &TestServer, path: &str) -> Vec<String> {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url(path));
    let lines = tab.run_until(|lines| lines.iter().any(|l| l == "done" || l.starts_with("error")));
    let origin = server.url("/").origin().serialization();
    lines
        .into_iter()
        .map(|l| l.replace(&origin, "ORIGIN"))
        .collect()
}

/// Probe p1's page, its `log` the console's and `report` a "done".
const P1_PAGE: &str = "<!doctype html><script>
const log = (s) => console.log(s); const report = () => console.log('done');
navigator.serviceWorker.addEventListener('message', e => log('msg ' + e.data));
const round = Number(sessionStorage.getItem('round') || '0');
sessionStorage.setItem('round', round + 1);
(async () => {
  if (round === 0) {
    log('secure ' + isSecureContext + ' has ' + ('serviceWorker' in navigator) + ' controller ' + navigator.serviceWorker.controller);
    log('protos ' + [typeof ServiceWorkerContainer, typeof ServiceWorkerRegistration, typeof ServiceWorker, typeof CacheStorage, typeof Cache].join(','));
    const p = navigator.serviceWorker.register('sw.js');
    log('register returned ' + Object.prototype.toString.call(p));
    const reg = await p;
    log('reg scope ' + reg.scope + ' updateViaCache ' + reg.updateViaCache + ' installing ' + (reg.installing && reg.installing.state) + ' waiting ' + reg.waiting + ' active ' + reg.active);
    const w = reg.installing;
    log('worker ' + w.scriptURL + ' ' + Object.prototype.toString.call(w));
    w.addEventListener('statechange', () => log('statechange ' + w.state));
    const ready = await navigator.serviceWorker.ready;
    log('ready same ' + (ready === reg) + ' active ' + ready.active.state + ' controller ' + navigator.serviceWorker.controller);
    const again = await navigator.serviceWorker.register('sw.js');
    log('register again same ' + (again === reg));
    const got = await navigator.serviceWorker.getRegistration();
    log('getRegistration same ' + (got === reg) + ' all ' + (await navigator.serviceWorker.getRegistrations()).length);
    setTimeout(() => location.reload(), 300);
  } else if (round === 1) {
    const c = navigator.serviceWorker.controller;
    log('round1 controller ' + (c && c.scriptURL) + ' state ' + (c && c.state));
    const reg = await navigator.serviceWorker.ready;
    log('controller is active ' + (c === reg.active));
    c.postMessage({hello: 1});
    setTimeout(() => { report(); }, 500);
  }
})().catch(e => { log('error ' + e.name + ': ' + e.message); report(); });
</script>";

/// Probe p1's worker.
const P1_WORKER: &str = "const tell = async (s) => { for (const c of await clients.matchAll({includeUncontrolled: true})) c.postMessage(s); };
self.addEventListener('install', e => {
  tell('install ' + (typeof e.waitUntil) + ' ' + Object.getPrototypeOf(e).constructor.name + ' state ' + registration.installing.state);
  e.waitUntil(new Promise(r => setTimeout(r, 100)).then(() => tell('install waited')));
});
self.addEventListener('activate', e => {
  tell('activate ' + Object.getPrototypeOf(e).constructor.name + ' state ' + registration.active.state);
});
self.addEventListener('message', async e => {
  const all = await clients.matchAll();
  e.source.postMessage('got ' + JSON.stringify(e.data) + ' source ' + Object.prototype.toString.call(e.source) + ' ' + e.source.type + ' frame ' + e.source.frameType + ' clients ' + all.length + ' url ' + e.source.url.replace(location.origin, ''));
});
tell('evaluated ' + Object.prototype.toString.call(self) + ' ' + [typeof clients, typeof registration, typeof skipWaiting, typeof caches, typeof serviceWorker, typeof importScripts, typeof fetch].join(','));";

#[test]
fn probe_p1_answers_as_the_reference_browser_did() {
    let server = serve(|path| match path {
        "/p1/index.html" => Some(html(P1_PAGE)),
        "/p1/sw.js" => Some(js(P1_WORKER)),
        _ => None,
    });
    let lines = run(&server, "/p1/index.html");
    let of = |keep: &dyn Fn(&str) -> bool| -> Vec<String> {
        lines.iter().filter(|l| keep(l)).cloned().collect()
    };
    assert_eq!(
        of(&|l| !l.starts_with("msg ") && !l.starts_with("statechange") && l != "done"),
        [
            "secure true has true controller null",
            "protos function,function,function,function,function",
            "register returned [object Promise]",
            "reg scope ORIGIN/p1/ updateViaCache imports installing installing waiting null active null",
            "worker ORIGIN/p1/sw.js [object ServiceWorker]",
            "ready same true active activating controller null",
            "register again same true",
            "getRegistration same true all 1",
            "round1 controller ORIGIN/p1/sw.js state activated",
            "controller is active true",
        ]
    );
    assert_eq!(
        of(&|l| l.starts_with("statechange")),
        [
            "statechange installed",
            "statechange activating",
            "statechange activated"
        ]
    );
    let mut messages = of(&|l| l.starts_with("msg "));
    messages.sort();
    assert_eq!(
        messages,
        [
            "msg activate ExtendableEvent state activating",
            "msg evaluated [object ServiceWorkerGlobalScope] object,object,function,object,object,function,function",
            "msg got {\"hello\":1} source [object WindowClient] window frame top-level clients 1 url /p1/index.html",
            "msg install function InstallEvent state installing",
            "msg install waited",
        ]
    );
    // The reply came last, after the controller's lines.
    let got = lines.iter().position(|l| l.starts_with("msg got")).unwrap();
    let round1 = lines.iter().position(|l| l.starts_with("round1")).unwrap();
    assert!(got > round1);
}

#[test]
fn a_new_version_claims_its_clients_and_they_hear_controllerchange() {
    // Probe p5's point: the controlled page's controller is the new
    // version once it skips waiting and claims.
    let page = "<!doctype html><script>
      const log = (s) => console.log(s);
      const ask = (w) => new Promise(r => { navigator.serviceWorker.onmessage = (e) => r(e.data); w.postMessage('v?'); });
      const round = Number(sessionStorage.getItem('round') || '0');
      sessionStorage.setItem('round', round + 1);
      (async () => {
        if (round === 0) {
          await navigator.serviceWorker.register('sw.js');
          await navigator.serviceWorker.ready;
          location.reload();
          return;
        }
        const reg = await navigator.serviceWorker.ready;
        log('controlled by V ' + await ask(navigator.serviceWorker.controller));
        const changed = new Promise(r => { navigator.serviceWorker.oncontrollerchange = r; });
        await reg.update();
        await changed;
        log('controllerchange, now V ' + await ask(navigator.serviceWorker.controller) + ' ' + navigator.serviceWorker.controller.state);
        log('done');
      })().catch(e => log('error ' + e.name + ': ' + e.message));
    </script>";
    let served = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = served.clone();
    let server = serve(move |path| match path {
        "/p5/index.html" => Some(html(page)),
        "/p5/sw.js" => {
            let n = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            Some(js(&format!(
                "self.V = {n};
                 self.addEventListener('install', () => skipWaiting());
                 self.addEventListener('activate', e => e.waitUntil(clients.claim()));
                 self.addEventListener('message', e => e.source.postMessage(self.V));"
            )))
        }
        _ => None,
    });
    let lines = run(&server, "/p5/index.html");
    assert_eq!(
        lines,
        [
            "controlled by V 1",
            "controllerchange, now V 2 activated",
            "done"
        ]
    );
}

#[test]
fn clients_lists_finds_and_refuses_in_the_reference_browsers_words() {
    // Probe p6's worker, from a controlled page.
    let page = "<!doctype html><script>
      const log = (s) => console.log(s);
      const round = Number(sessionStorage.getItem('round') || '0');
      sessionStorage.setItem('round', round + 1);
      (async () => {
        if (round === 0) {
          const reg = await navigator.serviceWorker.register('sw.js');
          await navigator.serviceWorker.ready;
          location.reload();
          return;
        }
        navigator.serviceWorker.onmessage = (e) => { log('clients ' + e.data); log('done'); };
        navigator.serviceWorker.controller.postMessage('who?');
      })().catch(e => log('error ' + e.name + ': ' + e.message));
    </script>";
    let worker = "self.addEventListener('install', e => e.waitUntil(clients.claim().then(() => 'claimed', x => x.name + ': ' + x.message).then(s => caches.open('c').then(c => c.put('/claim', new Response(s))))));
      self.addEventListener('activate', e => e.waitUntil(clients.claim()));
      self.addEventListener('message', async e => {
        const claim = await (await caches.match('/claim')).text();
        const all = await clients.matchAll({includeUncontrolled: true, type: 'all'});
        const one = await clients.get(e.source.id);
        let open; try { await clients.openWindow('/p6/x'); open = 'opened'; } catch (x) { open = x.name + ': ' + x.message; }
        let early; try { await new Clients(); } catch (x) { early = x.name; }
        e.source.postMessage(all.length + ' ' + all.map(c => c.type + ':' + c.frameType + ':' + c.url.replace(location.origin, '') + ':' + c.visibilityState + ':' + c.focused).join(',') + ' get same id ' + (one && one.id === e.source.id) + ' openWindow ' + open + ' focus ' + typeof e.source.focus + ' navigate ' + typeof e.source.navigate + ' new ' + early + ' id ' + /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(e.source.id) + ' early claim ' + claim);
      });";
    let server = serve(move |path| match path {
        "/p6/index.html" => Some(html(page)),
        "/p6/sw.js" => Some(js(worker)),
        _ => None,
    });
    let lines = run(&server, "/p6/index.html");
    assert_eq!(
        lines,
        [
            "clients 1 window:top-level:/p6/index.html:visible:true get same id true openWindow InvalidAccessError: Not allowed to open a window. focus function navigate function new TypeError id true early claim InvalidStateError: Only the active worker can claim clients.",
            "done"
        ]
    );
}
