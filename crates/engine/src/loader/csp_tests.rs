//! Content Security Policy's fetch directives (M33.1,
//! `docs/design/hardening.md`): every request a document makes held to
//! its destination's directive, report-only policies reporting, every
//! kind of document with its policies, a frame's navigations held to its
//! container's `frame-src`, a worker to `worker-src` and its own.

use net::Url;
use net::test_server::{Reply, TestServer};

use crate::testing::*;

/// Pages by path, each with its header fields and body.
type Pages = &'static [(
    &'static str,
    &'static [(&'static str, &'static str)],
    &'static str,
)];

/// A server of `pages` under two sites, `a.localhost` and `b.localhost`,
/// a query left aside; a body's `PORT` is the server's port.
fn sites(pages: Pages) -> TestServer {
    let port = std::sync::Arc::new(std::sync::OnceLock::<u16>::new());
    let seen = port.clone();
    let server = serve(move |target| {
        let path = target.split('?').next().unwrap_or_default();
        let (_, fields, body) = pages.iter().find(|(at, ..)| *at == path)?;
        let mut all = vec![("Content-Type", "text/html")];
        all.extend_from_slice(fields);
        let port = seen.get().map(u16::to_string).unwrap_or_default();
        let body = body.replace("PORT", &port);
        Some(Reply::with_fields(200, &all, body.as_bytes()))
    });
    let _ = port.set(server.url("/").port().expect("a port"));
    server
}

fn at(server: &TestServer, host: &str, path: &str) -> Url {
    let port = server.url("/").port().expect("a port");
    Url::parse(&format!("http://{host}:{port}{path}")).expect("a URL")
}

/// A tab with scripts on, `url` loaded, run until a line of the console
/// ends with `last`: the lines, the port written `PORT`, in order.
fn run_to(server: &TestServer, url: Url, last: &str) -> Vec<String> {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(url);
    let port = server.url("/").port().expect("a port").to_string();
    let mut lines: Vec<String> = tab
        .run_until(|lines| lines.iter().any(|line| line.ends_with(last)))
        .into_iter()
        .map(|line| line.replace(&port, "PORT"))
        .collect();
    lines.sort();
    lines
}

/// The paths of `server`'s requests.
fn asked(server: &TestServer) -> Vec<String> {
    server
        .requests()
        .into_iter()
        .map(|request| request.target)
        .collect()
}

#[test]
fn each_request_is_held_to_its_destinations_directive() {
    static PAGES: Pages = &[
        (
            "/page.html",
            &[(
                "Content-Security-Policy",
                "default-src 'self'; img-src 'none'; connect-src 'none'; \
                 script-src 'self' 'unsafe-inline'",
            )],
            "<link rel=stylesheet href='http://b.localhost:PORT/s.css'>\
             <link rel=stylesheet href=/own.css>\
             <img src=/i.png onerror=\"console.log('image error')\">\
             <iframe src='http://b.localhost:PORT/f.html' onload=\"console.log('frame load')\">\
             </iframe>\
             <script>\
             onload = () => fetch('/data').catch(e => console.log('fetch', e.name))\
               .then(() => console.log('loaded'));\
             </script>",
        ),
        (
            "/own.css",
            &[("Content-Type", "text/css")],
            "p { color: red }",
        ),
    ];
    let server = sites(PAGES);
    let lines = run_to(&server, at(&server, "a.localhost", "/page.html"), "loaded");
    let refused = |what: &str, directive: &str| {
        format!("Content-Security-Policy refused {what}: {directive}")
    };
    assert_eq!(
        lines,
        [
            refused(
                "the connection to http://a.localhost:PORT/data",
                "connect-src 'none'"
            ),
            refused(
                "the frame http://b.localhost:PORT/f.html",
                "default-src 'self'"
            ),
            refused(
                "the image http://a.localhost:PORT/favicon.ico",
                "img-src 'none'"
            ),
            refused("the image http://a.localhost:PORT/i.png", "img-src 'none'"),
            refused(
                "the stylesheet http://b.localhost:PORT/s.css",
                "default-src 'self'"
            ),
            "Failed to fetch http://a.localhost:PORT/data: blocked by \
             Content-Security-Policy: connect-src 'none'"
                .to_owned(),
            "fetch TypeError".to_owned(),
            "frame load".to_owned(),
            "image error".to_owned(),
            "loaded".to_owned(),
        ]
    );
    // None of them was asked for.
    let mut paths = asked(&server);
    paths.sort();
    assert_eq!(paths, ["/own.css", "/page.html"]);
}

#[test]
fn a_report_only_policy_reports_and_lets_everything_through() {
    static PAGES: Pages = &[
        (
            "/page.html",
            &[(
                "Content-Security-Policy-Report-Only",
                "img-src 'none'; script-src 'none'",
            )],
            "<img src=/i.png onerror=\"console.log('image done')\">\
             <script>console.log('ran'); onload = () => console.log('loaded')</script>",
        ),
        ("/i.png", &[("Content-Type", "image/png")], "not a png"),
    ];
    let server = sites(PAGES);
    let lines = run_to(&server, at(&server, "a.localhost", "/page.html"), "loaded");
    assert_eq!(
        lines,
        [
            "Content-Security-Policy (report-only) would refuse an event handler: \
             script-src 'none'",
            "Content-Security-Policy (report-only) would refuse an inline script: \
             script-src 'none'",
            "Content-Security-Policy (report-only) would refuse the image \
             http://a.localhost:PORT/favicon.ico: img-src 'none'",
            "Content-Security-Policy (report-only) would refuse the image \
             http://a.localhost:PORT/i.png: img-src 'none'",
            "image done",
            "loaded",
            "ran",
        ]
    );
    assert!(asked(&server).contains(&"/i.png".to_owned()));
}

#[test]
fn nonces_and_strict_dynamic_decide_external_scripts_and_stylesheets() {
    static PAGES: Pages = &[
        (
            "/page.html",
            &[(
                "Content-Security-Policy",
                "script-src 'nonce-abc' 'strict-dynamic'; style-src 'nonce-xyz'",
            )],
            "<link rel=stylesheet nonce=xyz href=/a.css>\
             <link rel=stylesheet href=/b.css>\
             <script nonce=abc src=/a.js></script>\
             <script src=/b.js></script>",
        ),
        ("/a.css", &[("Content-Type", "text/css")], ""),
        ("/b.css", &[("Content-Type", "text/css")], ""),
        (
            "/a.js",
            &[("Content-Type", "text/javascript")],
            "console.log('a');\
             const s = document.createElement('script');\
             s.src = '/c.js'; document.head.append(s);",
        ),
        (
            "/b.js",
            &[("Content-Type", "text/javascript")],
            "console.log('b')",
        ),
        (
            "/c.js",
            &[("Content-Type", "text/javascript")],
            "console.log('c')",
        ),
    ];
    let server = sites(PAGES);
    let lines = run_to(&server, at(&server, "a.localhost", "/page.html"), "c");
    assert_eq!(
        lines,
        [
            "Content-Security-Policy refused the script http://a.localhost:PORT/b.js: \
             script-src 'nonce-abc' 'strict-dynamic'",
            "Content-Security-Policy refused the stylesheet http://a.localhost:PORT/b.css: \
             style-src 'nonce-xyz'",
            "a",
            "c",
        ]
    );
}

#[test]
fn every_kind_of_document_and_a_javascript_urls_has_its_policies() {
    static PAGES: Pages = &[
        (
            "/image.svg",
            &[
                ("Content-Type", "image/svg+xml"),
                ("Content-Security-Policy", "script-src 'none'"),
            ],
            "<svg xmlns='http://www.w3.org/2000/svg'><script>console.log('ran')</script>\
             <image href='/after.png'/></svg>",
        ),
        (
            "/page.html",
            &[(
                "Content-Security-Policy",
                "script-src 'unsafe-inline'; img-src 'none'",
            )],
            "<a href=\"javascript:'<img src=/i.png><script>console.log(1)</script>'\">go</a>\
             <script>onload = () => document.querySelector('a').click()</script>",
        ),
    ];
    let server = sites(PAGES);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.load(at(&server, "a.localhost", "/image.svg"));
    let lines: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(
        lines,
        ["Content-Security-Policy refused an inline script: script-src 'none'"]
    );
    let lines = run_to(
        &server,
        at(&server, "a.localhost", "/page.html"),
        "i.png: img-src 'none'",
    );
    assert_eq!(
        lines.last().map(String::as_str),
        Some(
            "Content-Security-Policy refused the image http://a.localhost:PORT/i.png: \
             img-src 'none'"
        )
    );
    assert!(lines.contains(&"1".to_owned()));
    assert!(!asked(&server).contains(&"/i.png".to_owned()));
}

#[test]
fn a_frames_navigations_are_held_to_its_containers_frame_src() {
    static PAGES: Pages = &[
        (
            "/page.html",
            &[("Content-Security-Policy", "frame-src 'self'")],
            "<iframe src=/own.html></iframe>",
        ),
        (
            "/own.html",
            &[],
            "<script>console.log('own');\
             onload = () => location.href = 'http://b.localhost:PORT/away.html'</script>",
        ),
        ("/away.html", &[], "<script>console.log('away')</script>"),
    ];
    let server = sites(PAGES);
    let lines = run_to(&server, at(&server, "a.localhost", "/page.html"), "'self'");
    assert_eq!(
        lines,
        [
            "Content-Security-Policy refused the frame http://b.localhost:PORT/away.html: \
             frame-src 'self'",
            "[frame http://a.localhost:PORT/own.html] own",
        ]
    );
    assert!(!asked(&server).contains(&"/away.html".to_owned()));
}

#[test]
fn a_worker_is_held_to_worker_src_and_its_own_policies() {
    static PAGES: Pages = &[
        (
            "/refused.html",
            &[("Content-Security-Policy", "worker-src 'none'")],
            "<script>new Worker('/w.js').onerror = () => console.log('worker error')</script>",
        ),
        (
            "/page.html",
            &[],
            "<script>new Worker('/w.js').onmessage = e => console.log(e.data)</script>",
        ),
        (
            "/w.js",
            &[
                ("Content-Type", "text/javascript"),
                ("Content-Security-Policy", "connect-src 'none'"),
            ],
            "fetch('/data').then(() => postMessage('fetched'), e => postMessage(e.name))",
        ),
    ];
    let server = sites(PAGES);
    let lines = run_to(
        &server,
        at(&server, "a.localhost", "/refused.html"),
        "worker error",
    );
    assert_eq!(
        lines,
        [
            "Content-Security-Policy refused the worker http://a.localhost:PORT/w.js: \
             worker-src 'none'",
            "worker error",
        ]
    );
    let lines = run_to(
        &server,
        at(&server, "a.localhost", "/page.html"),
        "TypeError",
    );
    assert!(
        lines.contains(
            &"Content-Security-Policy refused the connection to \
              http://a.localhost:PORT/data: connect-src 'none'"
                .to_owned()
        ),
        "{lines:?}"
    );
    assert!(!asked(&server).contains(&"/data".to_owned()));
}

#[test]
fn a_meta_policy_holds_for_a_scripts_requests_before_the_document_is_shown() {
    static PAGES: Pages = &[(
        "/page.html",
        &[],
        "<head><meta http-equiv=Content-Security-Policy content=\"connect-src 'none'\">\
         <script>\
         const x = new XMLHttpRequest(); x.open('GET', '/sync', false);\
         try { x.send() } catch (e) { console.log('xhr', e.name) }\
         fetch('/data').catch(e => console.log('fetch', e.name));\
         </script>",
    )];
    let server = sites(PAGES);
    let lines = run_to(
        &server,
        at(&server, "a.localhost", "/page.html"),
        "fetch TypeError",
    );
    let refused = |path: &str| {
        format!(
            "Content-Security-Policy refused the connection to http://a.localhost:PORT/{path}: \
             connect-src 'none'"
        )
    };
    assert!(lines.contains(&refused("sync")), "{lines:?}");
    assert!(lines.contains(&refused("data")), "{lines:?}");
    assert!(lines.contains(&"xhr NetworkError".to_owned()), "{lines:?}");
    let paths = asked(&server);
    assert!(!paths.contains(&"/sync".to_owned()) && !paths.contains(&"/data".to_owned()));
}

/// The lines of `url` loaded in a tab with scripts on, until one ends
/// with `last`, in the order they came, the port written `PORT`.
fn run_in_order(server: &TestServer, url: Url, last: &str) -> Vec<String> {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(url);
    let port = server.url("/").port().expect("a port").to_string();
    tab.run_until(|lines| lines.iter().any(|line| line.ends_with(last)))
        .into_iter()
        .map(|line| line.replace(&port, "PORT"))
        .collect()
}

#[test]
fn an_inline_script_runs_by_its_nonce_or_its_hash_and_its_nonce_is_hidden() {
    static PAGES: Pages = &[(
        "/page.html",
        &[(
            "Content-Security-Policy",
            "script-src 'nonce-n1' 'sha256-Ly+EvtafdANT0xlOxh/fd3NF3f5hCoaFad0hrg25DRI='",
        )],
        "<script nonce=n1>const s = document.currentScript;\
             console.log('nonced', s.nonce, JSON.stringify(s.getAttribute('nonce')),\
               s.hasAttribute('nonce'), document.querySelectorAll('[nonce^=n]').length);\
             s.nonce = 'n2'; console.log(s.nonce, JSON.stringify(s.getAttribute('nonce')));\
             </script>\
             <script>console.log('hashed')</script>\
             <script>console.log('refused')</script>\
             <script nonce=n1 title='<script'>console.log('swallowed')</script>\
             <script nonce=' n1'>console.log('spaced')</script>\
             <script nonce=n1>console.log('done')</script>",
    )];
    let server = sites(PAGES);
    let lines = run_in_order(&server, at(&server, "a.localhost", "/page.html"), "done");
    let refused = "Content-Security-Policy refused an inline script: script-src 'nonce-n1' \
                   'sha256-Ly+EvtafdANT0xlOxh/fd3NF3f5hCoaFad0hrg25DRI='";
    assert_eq!(
        lines,
        [
            "nonced n1 \"\" true 0",
            "n2 \"\"",
            "hashed",
            refused,
            refused,
            "spaced",
            "done",
        ]
    );
}

#[test]
fn a_meta_policy_hides_no_nonce() {
    static PAGES: Pages = &[(
        "/page.html",
        &[],
        "<head><meta http-equiv=Content-Security-Policy content=\"script-src 'nonce-n1'\">\
         <script nonce=n1 id=first>console.log(document.currentScript.getAttribute('nonce'))\
         </script>\
         <script nonce=n1>console.log(document.getElementById('first').getAttribute('nonce'),\
           document.currentScript.getAttribute('nonce'))</script>",
    )];
    let server = sites(PAGES);
    let lines = run_in_order(&server, at(&server, "a.localhost", "/page.html"), "n1 n1");
    assert_eq!(lines, ["n1", "n1 n1"]);
}

#[test]
fn handlers_and_javascript_urls_need_unsafe_inline_or_their_hash_with_unsafe_hashes() {
    static PAGES: Pages = &[(
        "/page.html",
        &[(
            "Content-Security-Policy",
            "script-src 'nonce-n1' 'unsafe-hashes' \
             'sha256-mKO3dbAQavALIF8PI6Y9pSmdsqPKu5KIQ7wIb73GgI0=' \
             'sha256-vojPZRgpYTJJDqV9v1ys7HwKq7QYV+VROTr47k0NH50='",
        )],
        "<button onclick=\"console.log('clicked')\">b</button>\
         <div onclick=\"console.log('div')\">d</div>\
         <a id=bad href=\"javascript:console.log('js')\">bad</a>\
         <a id=ok href=\"javascript:console.log('ok')\">ok</a>\
         <script nonce=n1>\
         document.querySelector('button').click();\
         document.querySelector('div').click();\
         setTimeout(() => {\
           document.getElementById('bad').click();\
           setTimeout(() => document.getElementById('ok').click(), 10);\
         }, 10);\
         </script>",
    )];
    let server = sites(PAGES);
    let lines = run_in_order(&server, at(&server, "a.localhost", "/page.html"), "ok");
    let policy = "script-src 'nonce-n1' 'unsafe-hashes' \
                  'sha256-mKO3dbAQavALIF8PI6Y9pSmdsqPKu5KIQ7wIb73GgI0=' \
                  'sha256-vojPZRgpYTJJDqV9v1ys7HwKq7QYV+VROTr47k0NH50='";
    assert_eq!(
        lines,
        [
            "clicked".to_owned(),
            format!("Content-Security-Policy refused an event handler: {policy}"),
            format!("Content-Security-Policy refused a javascript: URL: {policy}"),
            "ok".to_owned(),
        ]
    );
}

#[test]
fn code_from_strings_needs_unsafe_eval() {
    static PAGES: Pages = &[
        (
            "/page.html",
            &[("Content-Security-Policy", "script-src 'nonce-n1'")],
            "<script nonce=n1>\
             const t = f => { try { return String(f()); } catch (e) { return e.name; } };\
             console.log(t(() => eval('1')), t(() => (0, eval)('2')),\
               t(() => Function('return 3')()), t(() => new Function('return 4')()),\
               t(() => (async function () {}).constructor('return 5')),\
               t(() => (function* () {}).constructor('yield 6')));\
             console.log(eval === window.eval, Function.prototype.constructor === Function,\
               (() => 1) instanceof Function, Function.name, Function.length, eval.name,\
               typeof setTimeout('window.x = 1'), typeof eval({}));\
             try { eval('1') } catch (e) { console.log(e.message) }\
             setTimeout(() => console.log('x', window.x), 20);\
             </script>",
        ),
        (
            "/allowed.html",
            &[(
                "Content-Security-Policy",
                "script-src 'nonce-n1' 'unsafe-eval'",
            )],
            "<script nonce=n1>const local = 7; console.log(eval('local'), Function('return 8')())\
             </script>",
        ),
        (
            "/reported.html",
            &[("Content-Security-Policy-Report-Only", "script-src 'none'")],
            "<script>console.log(eval('1 + 1'), Function('return 3')())</script>",
        ),
    ];
    let server = sites(PAGES);
    let lines = run_in_order(
        &server,
        at(&server, "a.localhost", "/page.html"),
        "x undefined",
    );
    let refused = "Content-Security-Policy refused a string as code: script-src 'nonce-n1'";
    assert_eq!(
        lines,
        [
            refused,
            refused,
            refused,
            refused,
            refused,
            refused,
            "EvalError EvalError EvalError EvalError EvalError EvalError",
            refused,
            "true true true Function 1 eval number object",
            refused,
            "Evaluating a string as JavaScript violates the following Content Security Policy \
             directive because 'unsafe-eval' is not an allowed source of script: script-src \
             'nonce-n1'\".\n",
            "x undefined",
        ]
    );
    let lines = run_in_order(&server, at(&server, "a.localhost", "/allowed.html"), "8");
    assert_eq!(lines, ["7 8"]);
    let lines = run_in_order(&server, at(&server, "a.localhost", "/reported.html"), "2 3");
    assert_eq!(
        lines,
        [
            "Content-Security-Policy (report-only) would refuse an inline script: \
             script-src 'none'",
            "Content-Security-Policy (report-only) would refuse a string as code: \
             script-src 'none'",
            "Content-Security-Policy (report-only) would refuse a string as code: \
             script-src 'none'",
            "2 3",
        ]
    );
}

#[test]
fn a_webassembly_module_needs_wasm_unsafe_eval() {
    static PAGES: Pages = &[
        (
            "/page.html",
            &[("Content-Security-Policy", "script-src 'nonce-n1'")],
            "<script nonce=n1>\
             const bytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]);\
             try { new WebAssembly.Module(bytes); } catch (e) { console.log(e.name, e.message) }\
             console.log(WebAssembly.validate(bytes));\
             </script>",
        ),
        (
            "/allowed.html",
            &[(
                "Content-Security-Policy",
                "script-src 'nonce-n1' 'wasm-unsafe-eval'",
            )],
            "<script nonce=n1>\
             const bytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]);\
             console.log(new WebAssembly.Module(bytes) instanceof WebAssembly.Module);\
             try { eval('1') } catch (e) { console.log(e.name) }\
             </script>",
        ),
    ];
    let server = sites(PAGES);
    let lines = run_in_order(&server, at(&server, "a.localhost", "/page.html"), "true");
    assert_eq!(
        lines,
        [
            "Content-Security-Policy refused a WebAssembly module: script-src 'nonce-n1'",
            "CompileError WebAssembly.Module(): Compiling or instantiating WebAssembly module \
             violates the following Content Security policy directive because 'unsafe-eval' is \
             not an allowed source of script in the following Content Security Policy \
             directive: \"script-src 'nonce-n1'\".",
            "true",
        ]
    );
    let lines = run_in_order(
        &server,
        at(&server, "a.localhost", "/allowed.html"),
        "EvalError",
    );
    assert_eq!(
        lines,
        [
            "true",
            "Content-Security-Policy refused a string as code: script-src 'nonce-n1' \
             'wasm-unsafe-eval'",
            "EvalError",
        ]
    );
}

#[test]
fn a_workers_own_policies_hold_for_its_code() {
    static PAGES: Pages = &[
        (
            "/page.html",
            &[],
            "<script>new Worker('/w.js').onmessage = e => console.log(e.data)</script>",
        ),
        (
            "/w.js",
            &[
                ("Content-Type", "text/javascript"),
                ("Content-Security-Policy", "script-src 'self'"),
            ],
            "try { eval('1') } catch (e) { postMessage(e.name) }",
        ),
    ];
    let server = sites(PAGES);
    let lines = run_in_order(
        &server,
        at(&server, "a.localhost", "/page.html"),
        "EvalError",
    );
    assert_eq!(
        lines,
        [
            "Content-Security-Policy refused a string as code: script-src 'self'",
            "EvalError",
        ]
    );
}

#[test]
fn a_nonce_a_script_gives_an_element_it_inserts_is_hidden_too() {
    static PAGES: Pages = &[(
        "/page.html",
        &[("Content-Security-Policy", "script-src 'nonce-n1'")],
        "<script nonce=n1>onload = () => {\
           const p = document.createElement('p');\
           p.setAttribute('nonce', 'n7');\
           document.body.append(p);\
           setTimeout(() => console.log('later', JSON.stringify(p.getAttribute('nonce')),\
             p.nonce), 0);\
         };</script>",
    )];
    let server = sites(PAGES);
    let lines = run_in_order(&server, at(&server, "a.localhost", "/page.html"), "n7");
    assert_eq!(lines, ["later \"\" n7"]);
}
