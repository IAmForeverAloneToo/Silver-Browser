//! Service workers in one process (M39): registering, the lifecycle and
//! the objects a page and a worker see, as the reference browser
//! answered the same calls (scratchpad `m39/pages/p1`, `p4`, `p5`,
//! `p6`; Chrome 155), the worker's own lines kept in a cache, since it
//! has no clients to tell yet.

use net::test_server::{Reply, TestServer};

use crate::testing::*;

/// A page whose script runs `body` (an async function's) and logs
/// "done" after it; `log` writes a line.
fn page(body: &str) -> String {
    format!(
        "<script>\
         const log = (s) => console.log(s);\
         const tryit = async (name, f) => {{ try {{ const v = await f(); log(name + ': ' + v); }} \
         catch (e) {{ log(name + ': ' + e.name + ': ' + e.message); }} }};\
         const sleep = (ms) => new Promise(r => setTimeout(r, ms));\
         const swlog = async () => {{ const c = await caches.open('log'); const out = []; \
         for (const k of await c.keys()) out.push(await (await c.match(k)).text()); return out; }};\
         (async () => {{ {body} }})().then(() => log('done'), (e) => log('failed ' + e.name + ': ' + e.message));\
         </script>"
    )
}

/// A worker's script that keeps its lines in the cache `log`, in order,
/// then runs `body`.
fn worker(body: &str) -> String {
    format!(
        "let n = 0;\
         const log = (s) => {{ const at = '/log/' + String(n++).padStart(3, '0'); \
         return caches.open('log').then(c => c.put(at, new Response(s))); }};\
         {body}"
    )
}

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

fn run(server: &TestServer, path: &str) -> Vec<String> {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url(path));
    let lines = tab.run_until(|lines| lines.iter().any(|l| l == "done" || l.starts_with("failed")));
    let origin = server.url("/").origin().serialization();
    lines
        .into_iter()
        .filter(|l| l != "done")
        .map(|l| l.replace(&origin, "ORIGIN"))
        .collect()
}

#[test]
fn a_worker_registers_installs_and_activates_as_the_reference_browsers_does() {
    let body = page(
        "log('secure ' + isSecureContext + ' has ' + ('serviceWorker' in navigator) + ' controller ' + navigator.serviceWorker.controller);\
         log('protos ' + [typeof ServiceWorkerContainer, typeof ServiceWorkerRegistration, typeof ServiceWorker, typeof CacheStorage, typeof Cache].join(','));\
         const p = navigator.serviceWorker.register('sw.js');\
         log('register returned ' + Object.prototype.toString.call(p));\
         const reg = await p;\
         log('reg scope ' + reg.scope + ' updateViaCache ' + reg.updateViaCache + ' installing ' + (reg.installing && reg.installing.state) + ' waiting ' + reg.waiting + ' active ' + reg.active);\
         const w = reg.installing;\
         log('worker ' + w.scriptURL + ' ' + Object.prototype.toString.call(w));\
         const states = [];\
         w.addEventListener('statechange', () => states.push(w.state));\
         const ready = await navigator.serviceWorker.ready;\
         log('ready same ' + (ready === reg) + ' active ' + ready.active.state + ' controller ' + navigator.serviceWorker.controller);\
         const again = await navigator.serviceWorker.register('sw.js');\
         log('register again same ' + (again === reg));\
         const got = await navigator.serviceWorker.getRegistration();\
         log('getRegistration same ' + (got === reg) + ' all ' + (await navigator.serviceWorker.getRegistrations()).length);\
         for (let i = 0; i < 100 && !states.includes('activated'); i++) await sleep(20);\
         log('statechanges ' + states.join(','));\
         for (const line of await swlog()) log('sw ' + line);",
    );
    let sw = worker(
        "self.addEventListener('install', e => {\
           log('install ' + (typeof e.waitUntil) + ' ' + Object.getPrototypeOf(e).constructor.name + ' state ' + registration.installing.state);\
           e.waitUntil(new Promise(r => setTimeout(r, 100)).then(() => log('install waited')));\
         });\
         self.addEventListener('activate', e => {\
           log('activate ' + Object.getPrototypeOf(e).constructor.name + ' state ' + registration.active.state);\
         });\
         log('evaluated ' + Object.prototype.toString.call(self) + ' ' + [typeof registration, typeof skipWaiting, typeof caches, typeof serviceWorker, typeof importScripts, typeof fetch].join(',') + ' ' + serviceWorker.state + ' ' + (typeof XMLHttpRequest) + ' ' + (typeof Worker) + ' ' + (typeof postMessage));",
    );
    let server = serve(move |path| match path {
        "/p1/index.html" => Some(html(&body)),
        "/p1/sw.js" => Some(js(&sw)),
        _ => None,
    });
    assert_eq!(
        run(&server, "/p1/index.html"),
        [
            "secure true has true controller null",
            "protos function,function,function,function,function",
            "register returned [object Promise]",
            "reg scope ORIGIN/p1/ updateViaCache imports installing installing waiting null active null",
            "worker ORIGIN/p1/sw.js [object ServiceWorker]",
            "ready same true active activating controller null",
            "register again same true",
            "getRegistration same true all 1",
            "statechanges installed,activating,activated",
            "sw evaluated [object ServiceWorkerGlobalScope] object,function,object,object,function,function parsed undefined undefined undefined",
            "sw install function InstallEvent state installing",
            "sw install waited",
            "sw activate ExtendableEvent state activating",
        ]
    );
}

#[test]
fn a_registration_fails_in_the_reference_browsers_words() {
    let other = serve(|_| None);
    let elsewhere = other.url("/").origin().serialization();
    let body = page(&format!(
        "const reg = (s, o) => navigator.serviceWorker.register(s, o).then(r => r.scope.replace(location.origin, ''));\
         await tryit('scope above script', () => reg('sub/sw.js', {{scope: './'}}));\
         await tryit('allowed header', () => reg('sub/sw.js?allowed=/p4/', {{scope: './'}}));\
         await tryit('scope default', () => reg('sub/sw.js'));\
         await tryit('wrong type', () => reg('sw.js?type=text/plain', {{scope: 'x/'}}));\
         await tryit('html type', () => reg('sw.js?type=text/html', {{scope: 'y/'}}));\
         await tryit('404', () => reg('none.js', {{scope: 'z/'}}));\
         await tryit('500', () => reg('sw.js?status=500', {{scope: 'w/'}}));\
         await tryit('syntax', () => reg('bad.js', {{scope: 'v/'}}));\
         await tryit('throws', () => reg('throws.js', {{scope: 'u/'}}));\
         await tryit('cross origin', () => reg('{elsewhere}/sw.js'));\
         await tryit('cross scope', () => reg('sw.js', {{scope: '{elsewhere}/'}}));\
         await tryit('data url', () => reg('data:text/javascript,1'));\
         await tryit('bad url', () => reg('http://[::1/'));\
         await tryit('scope with %2f', () => reg('sw.js', {{scope: 'a%2fb/'}}));\
         await tryit('bad type option', () => reg('sw.js', {{type: 'x'}}));\
         await tryit('updateViaCache none', () => navigator.serviceWorker.register('sw.js', {{scope: 'q/', updateViaCache: 'none'}}).then(r => r.updateViaCache));\
         await tryit('getRegistration q/deep', () => navigator.serviceWorker.getRegistration('q/deep/x').then(r => r.scope.replace(location.origin, '')));\
         await tryit('getRegistration none', () => navigator.serviceWorker.getRegistration('/elsewhere/'));\
         await tryit('getRegistration cross', () => navigator.serviceWorker.getRegistration('{elsewhere}/'));\
         await tryit('registrations', async () => (await navigator.serviceWorker.getRegistrations()).map(r => r.scope.replace(location.origin, '')).join(','));\
         await tryit('unregister', async () => {{ const r = await navigator.serviceWorker.getRegistration('q/'); return [await r.unregister(), await r.unregister(), Object.prototype.toString.call(r)].join(','); }});",
    ));
    let server = serve(move |path| match path {
        "/p4/index.html" => Some(html(&body)),
        "/p4/bad.js" => Some(js("this is not javascript")),
        "/p4/throws.js" => Some(js("throw new Error('no')")),
        _ if path.starts_with("/p4/sw.js") || path.starts_with("/p4/sub/sw.js") => {
            let query = path.split_once('?').map_or("", |(_, q)| q);
            let param = |name: &str| {
                query
                    .split('&')
                    .find_map(|pair| pair.strip_prefix(&format!("{name}=")))
                    .map(str::to_owned)
            };
            let status: u16 = param("status").and_then(|s| s.parse().ok()).unwrap_or(200);
            let kind = param("type").unwrap_or_else(|| "text/javascript".to_owned());
            let mut fields = vec![
                ("Content-Type", kind.clone()),
                ("Cache-Control", "no-store".to_owned()),
            ];
            if let Some(allowed) = param("allowed") {
                fields.push(("Service-Worker-Allowed", allowed));
            }
            let fields: Vec<(&str, &str)> = fields.iter().map(|(n, v)| (*n, v.as_str())).collect();
            Some(Reply::with_fields(
                status,
                &fields,
                b"self.oninstall = () => {};",
            ))
        }
        _ => None,
    });
    let lines = run(&server, "/p4/index.html");
    let lines: Vec<String> = lines
        .into_iter()
        .map(|l| l.replace(&elsewhere, "ELSEWHERE"))
        .collect();
    assert_eq!(
        lines,
        [
            "scope above script: SecurityError: Failed to register a ServiceWorker for scope ('ORIGIN/p4/') with script ('ORIGIN/p4/sub/sw.js'): The path of the provided scope ('/p4/') is not under the max scope allowed ('/p4/sub/'). Adjust the scope, move the Service Worker script, or use the Service-Worker-Allowed HTTP header to allow the scope.",
            "allowed header: /p4/",
            "scope default: /p4/sub/",
            "wrong type: SecurityError: Failed to register a ServiceWorker for scope ('ORIGIN/p4/x/') with script ('ORIGIN/p4/sw.js?type=text/plain'): The script has an unsupported MIME type ('text/plain').",
            "html type: SecurityError: Failed to register a ServiceWorker for scope ('ORIGIN/p4/y/') with script ('ORIGIN/p4/sw.js?type=text/html'): The script has an unsupported MIME type ('text/html').",
            "404: TypeError: Failed to register a ServiceWorker for scope ('ORIGIN/p4/z/') with script ('ORIGIN/p4/none.js'): A bad HTTP response code (404) was received when fetching the script.",
            "500: TypeError: Failed to register a ServiceWorker for scope ('ORIGIN/p4/w/') with script ('ORIGIN/p4/sw.js?status=500'): A bad HTTP response code (500) was received when fetching the script.",
            "syntax: TypeError: Failed to register a ServiceWorker for scope ('ORIGIN/p4/v/') with script ('ORIGIN/p4/bad.js'): ServiceWorker script evaluation failed",
            "throws: TypeError: Failed to register a ServiceWorker for scope ('ORIGIN/p4/u/') with script ('ORIGIN/p4/throws.js'): ServiceWorker script evaluation failed",
            "cross origin: SecurityError: Failed to register a ServiceWorker: The origin of the provided scriptURL ('ELSEWHERE') does not match the current origin ('ORIGIN').",
            "cross scope: SecurityError: Failed to register a ServiceWorker: The origin of the provided scope ('ELSEWHERE') does not match the current origin ('ORIGIN').",
            "data url: TypeError: Failed to register a ServiceWorker: The URL protocol of the script ('data:text/javascript,1') is not supported.",
            "bad url: TypeError: Failed to register a ServiceWorker: The URL protocol of the script ('http://[::1/') is not supported.",
            "scope with %2f: TypeError: Failed to register a ServiceWorker: The provided scope ('ORIGIN/p4/a%2fb/') or scriptURL ('ORIGIN/p4/sw.js') includes a disallowed escape character.",
            "bad type option: TypeError: Failed to execute 'register' on 'ServiceWorkerContainer': Failed to read the 'type' property from 'RegistrationOptions': The provided value 'x' is not a valid enum value of type WorkerType.",
            "updateViaCache none: none",
            "getRegistration q/deep: /p4/q/",
            "getRegistration none: undefined",
            "getRegistration cross: SecurityError: Failed to get a ServiceWorkerRegistration: The origin of the provided documentURL ('ELSEWHERE') does not match the current origin ('ORIGIN').",
            "registrations: /p4/,/p4/q/,/p4/sub/",
            "unregister: true,false,[object ServiceWorkerRegistration]",
        ]
    );
}

#[test]
fn an_update_installs_a_version_that_waits_until_it_skips_waiting() {
    // M39 (probe p5): the page's registration hears `updatefound` and
    // the new version's states; it waits while the registration is in
    // use, and `skipWaiting()` activates it.
    let body = page(
        "const st = (reg) => ['installing', 'waiting', 'active'].map(k => k + '=' + (reg[k] ? reg[k].state : '-')).join(' ');\
         const reg = await navigator.serviceWorker.register('sw.js');\
         await navigator.serviceWorker.ready;\
         for (let i = 0; i < 100 && reg.active.state !== 'activated'; i++) await sleep(20);\
         log('first ' + st(reg));\
         reg.onupdatefound = () => log('updatefound ' + st(reg));\
         const u = await reg.update();\
         log('update resolved same ' + (u === reg) + ' ' + st(reg));\
         const first = reg.active;\
         for (let i = 0; i < 100 && (reg.active === first || reg.active.state !== 'activated'); i++) await sleep(20);\
         log('after ' + st(reg) + ' new ' + (reg.active !== first) + ' old ' + first.state);",
    );
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let served = count.clone();
    let server = serve(move |path| match path {
        "/p5/index.html" => Some(html(&body)),
        "/p5/sw.js" => {
            let n = served.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            // The second version skips waiting at its install.
            Some(js(&format!(
                "const v = {n}; self.oninstall = () => {{ if (v > 0) skipWaiting(); }};"
            )))
        }
        _ => None,
    });
    let lines = run(&server, "/p5/index.html");
    assert_eq!(lines[0], "first installing=- waiting=- active=activated");
    assert_eq!(
        lines[1],
        "update resolved same true installing=installing waiting=- active=activated"
    );
    assert_eq!(
        lines[2],
        "updatefound installing=installing waiting=- active=activated"
    );
    assert_eq!(
        lines[3],
        "after installing=- waiting=- active=activated new true old redundant"
    );
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[test]
fn an_insecure_or_sandboxed_context_has_no_service_workers() {
    let body = page(
        "const frame = (attrs, src) => new Promise(r => { const f = document.createElement('iframe');\
           for (const [k, v] of Object.entries(attrs)) f.setAttribute(k, v); f.srcdoc = src;\
           onmessage = e => r(e.data); document.documentElement.append(f); });\
         log('sandboxed ' + await frame({sandbox: 'allow-scripts'},\
           '<script>let s; try { navigator.serviceWorker; s = \"no throw\"; } catch (e) { s = e.name + \": \" + e.message; } parent.postMessage(s, \"*\")<\\/script>'));",
    );
    let server = serve(move |path| (path == "/page.html").then(|| html(&body)));
    assert_eq!(
        run(&server, "/page.html"),
        [
            "sandboxed SecurityError: Failed to read the 'serviceWorker' property from 'Navigator': Service worker is disabled because the context is sandboxed and lacks the 'allow-same-origin' flag."
        ]
    );
    let insecure = serve(|path| {
        (path == "/page.html").then(|| {
            html(
                "<script>console.log([isSecureContext, typeof navigator.serviceWorker, \
                 'serviceWorker' in navigator, typeof ServiceWorkerContainer].join(' '));</script>",
            )
        })
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.set_network(net::Client::new(
        net::transport::Config::default()
            .resolve("insecure.test", std::net::IpAddr::from([127, 0, 0, 1])),
    ));
    let url = net::Url::parse(&format!(
        "http://insecure.test:{}/page.html",
        insecure.port()
    ))
    .unwrap();
    tab.engine.navigate(url);
    let lines = tab.run_until(|lines| !lines.is_empty());
    assert_eq!(lines, ["false undefined false undefined"]);
}

#[test]
fn a_rejected_wait_until_fails_the_install() {
    let body = page(
        "const reg = await navigator.serviceWorker.register('fails.js', {scope: 'f/'});\
         const w = reg.installing;\
         for (let i = 0; i < 100 && w.state !== 'redundant'; i++) await sleep(20);\
         log('state ' + w.state + ' installing ' + reg.installing + ' kept ' + (await navigator.serviceWorker.getRegistration('f/')));\
         let thrown;\
         try { new ExtendableEvent('x').waitUntil(Promise.resolve()); } catch (e) { thrown = e.name; }\
         log('constructed ' + typeof ExtendableEvent + ' ' + thrown);",
    );
    let server = serve(move |path| match path {
        "/page.html" => Some(html(&body)),
        "/fails.js" => Some(js(
            "self.oninstall = (e) => e.waitUntil(Promise.reject(new Error('no')));",
        )),
        _ => None,
    });
    assert_eq!(
        run(&server, "/page.html"),
        [
            "state redundant installing null kept undefined",
            // ExtendableEvent is a service worker's alone.
            "constructed undefined ReferenceError",
        ]
    );
}
