//! Preloads (M38): what the scanner finds past a script that blocks the
//! parser is asked for at once, and the page's own requests take those
//! answers rather than asking again; `<link rel=preload>`, the console's
//! word for one nothing took, the bound, and the policies a preload is
//! held to.

use std::sync::Arc;
use std::time::{Duration, Instant};

use net::test_server::{Reply, TestServer};

use super::MAX_PRELOADS;
use super::tests::load_all;
use crate::LoadState;
use crate::testing::*;

/// Waits for the navigation to `url` under way to load.
fn finish(tab: &mut Tab, url: &net::Url) {
    tab.wait_until(|engine| {
        incoming(engine).is_none()
            && engine.url() == Some(url)
            && engine.load_state() == &LoadState::Loaded
    });
}

/// Lets the engine take what comes for `ms` milliseconds.
fn idle(tab: &mut Tab, ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        tab.engine.poll_network();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn javascript(body: &str) -> Reply {
    Reply::ok("text/javascript", body.as_bytes())
}

fn png() -> Reply {
    Reply::ok("image/png", b"")
}

/// How many requests for `target` reached `server`.
fn reached(server: &TestServer, target: &str) -> usize {
    server
        .requests()
        .iter()
        .filter(|request| request.target == target)
        .count()
}

/// A server whose `/page.html` is `page`, its `/slow.js` held until
/// `gate` opens, and any other path a script, sheet or image by its
/// extension.
fn gated(page: &'static str, gate: &Arc<Gate>) -> TestServer {
    let gate = gate.clone();
    serve(move |path| {
        let path = path.split('?').next().unwrap_or(path);
        if path == "/page.html" {
            return Some(html(page));
        }
        if path == "/slow.js" {
            gate.wait();
            return Some(javascript("console.log('slow')"));
        }
        if path.ends_with(".js") {
            return Some(javascript(&format!("console.log('{path}')")));
        }
        if path.ends_with(".css") {
            return Some(css(".a { color: red }"));
        }
        path.ends_with(".png").then(png)
    })
}

const BLOCKED: &str = "<script src=slow.js></script>\
    <link rel=stylesheet href=after.css><script src=after.js></script>\
    <img src=after.png><p class=a>x</p>";

#[test]
fn what_follows_a_blocking_script_is_asked_for_before_it_comes_and_once() {
    let gate = Gate::new();
    let server = gated(BLOCKED, &gate);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    let deadline = Instant::now() + WAIT;
    while ["/after.css", "/after.js", "/after.png"]
        .iter()
        .any(|path| reached(&server, path) == 0)
    {
        assert!(Instant::now() < deadline, "preloaded: {:?}", paths(&server));
        tab.engine.poll_network();
        std::thread::sleep(Duration::from_millis(2));
    }
    gate.open();
    finish(&mut tab, &server.url("/page.html"));
    for path in ["/slow.js", "/after.css", "/after.js", "/after.png"] {
        assert_eq!(reached(&server, path), 1, "{path}");
    }
    let lines: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(lines, ["slow", "/after.js"]);
    assert_eq!(tab.names(), ["a"], "the sheet applied");
}

#[test]
fn without_the_scanner_nothing_past_the_script_is_asked_for() {
    let gate = Gate::new();
    let server = gated(BLOCKED, &gate);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.set_preload_scanner(false);
    tab.engine.navigate(server.url("/page.html"));
    let deadline = Instant::now() + WAIT;
    while reached(&server, "/slow.js") == 0 {
        assert!(Instant::now() < deadline);
        tab.engine.poll_network();
        std::thread::sleep(Duration::from_millis(2));
    }
    idle(&mut tab, 300);
    assert_eq!(reached(&server, "/after.js"), 0);
    assert_eq!(reached(&server, "/after.png"), 0);
    gate.open();
    finish(&mut tab, &server.url("/page.html"));
    assert_eq!(reached(&server, "/after.js"), 1);
}

#[test]
fn a_link_preload_is_taken_by_its_request_and_one_nothing_took_is_said() {
    let gate = Gate::new();
    let server = gated(
        "<link rel=preload href=used.png as=image>\
         <link rel=preload href=unused.js as=script>\
         <link rel=preload href=font.woff as=font>\
         <link rel=preload href=cors.png as=image crossorigin>\
         <img src=used.png><img src=cors.png>",
        &gate,
    );
    gate.open();
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    // A preload nothing took may still be on its way at the load.
    tab.wait_until(|_| reached(&server, "/unused.js") >= 1 && reached(&server, "/cors.png") >= 2);
    assert_eq!(
        reached(&server, "/used.png"),
        1,
        "taken: {:?}",
        server
            .requests()
            .iter()
            .map(|request| request.target.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(reached(&server, "/unused.js"), 1);
    assert_eq!(reached(&server, "/font.woff"), 0, "no font is fetched");
    // A CORS preload is not a plain image's: asked again.
    assert_eq!(reached(&server, "/cors.png"), 2);
    let warnings: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .filter(|entry| entry.level == js::ConsoleLevel::Warn)
        .map(|entry| entry.text.replace(&server.url("/").to_string(), "/"))
        .collect();
    assert_eq!(
        warnings,
        [
            "The resource /cors.png was preloaded using link preload but not used by the \
             window's load event.",
            "The resource /unused.js was preloaded using link preload but not used by the \
             window's load event."
        ]
    );
}

#[test]
fn a_link_preload_a_script_made_is_preloaded_too() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<script>const add = (href) => { const l = document.createElement('link');\
             l.rel = 'preload'; l.as = 'image'; l.href = href;\
             document.head.append(l); };\
             add('made.png'); onload = () => { const l = document.createElement('link');\
             l.rel = 'preload'; l.href = 'later.png'; document.head.append(l);\
             setTimeout(() => { l.as = 'image'; }, 0); };</script><p>x</p>",
        )),
        "/made.png" | "/later.png" => Some(png()),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    // While the page parsed; and after it loaded, its `as` given in a
    // later task, once it was in the tree.
    let deadline = Instant::now() + WAIT;
    while reached(&server, "/later.png") == 0 {
        assert!(Instant::now() < deadline, "{:?}", paths(&server));
        tab.engine.run_tasks();
        tab.engine.poll_network();
        std::thread::sleep(Duration::from_millis(2));
    }
    tab.settle();
    assert_eq!(reached(&server, "/made.png"), 1);
    assert_eq!(reached(&server, "/later.png"), 1);
}

#[test]
fn a_preload_carries_its_elements_nonce() {
    // M38: under a nonce policy, the script with the nonce is preloaded
    // past the one that blocks; the one without is not, nor fetched.
    let gate = Gate::new();
    let server = {
        let gate = gate.clone();
        serve(move |path| match path {
            "/page.html" => Some(Reply::with_fields(
                200,
                &[
                    ("Content-Type", "text/html"),
                    ("Cache-Control", "no-store"),
                    ("Content-Security-Policy", "script-src 'nonce-abc'"),
                ],
                b"<script nonce=abc src=slow.js></script>\
                  <script nonce=abc src=after.js></script><script src=bare.js></script>",
            )),
            "/slow.js" => {
                gate.wait();
                Some(javascript("console.log('slow')"))
            }
            "/after.js" | "/bare.js" => Some(javascript("console.log('after')")),
            _ => None,
        })
    };
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    let deadline = Instant::now() + WAIT;
    while reached(&server, "/after.js") == 0 {
        assert!(Instant::now() < deadline, "{:?}", paths(&server));
        tab.engine.poll_network();
        std::thread::sleep(Duration::from_millis(2));
    }
    gate.open();
    finish(&mut tab, &server.url("/page.html"));
    assert_eq!(reached(&server, "/after.js"), 1);
    assert_eq!(reached(&server, "/bare.js"), 0);
}

#[test]
fn a_page_preloads_at_most_its_bound() {
    let gate = Gate::new();
    let images: String = (0..MAX_PRELOADS + 40)
        .map(|n| format!("<img src=i{n}.png>"))
        .collect();
    let page: &'static str =
        Box::leak(format!("<script src=slow.js></script>{images}").into_boxed_str());
    let server = gated(page, &gate);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    let images = |server: &TestServer| {
        server
            .requests()
            .iter()
            .filter(|request| request.target.ends_with(".png"))
            .count()
    };
    // The script is one of the page's preloads.
    let deadline = Instant::now() + WAIT;
    while images(&server) < MAX_PRELOADS - 1 {
        assert!(Instant::now() < deadline, "{}", images(&server));
        tab.engine.poll_network();
        std::thread::sleep(Duration::from_millis(2));
    }
    idle(&mut tab, 200);
    assert_eq!(images(&server), MAX_PRELOADS - 1);
    gate.open();
    finish(&mut tab, &server.url("/page.html"));
    assert_eq!(images(&server), MAX_PRELOADS + 40, "each once");
}

#[test]
fn a_meta_policy_stops_the_preloads_and_a_refused_one_is_never_made() {
    let other = serve(|_| Some(png()));
    let elsewhere = other.url("/x.png").to_string();
    let gate = Gate::new();
    let page: &'static str = Box::leak(
        format!(
            "<base href=/b/><script src=/slow.js></script><link rel=stylesheet href=based.css>\
             <meta http-equiv=Content-Security-Policy content=\"img-src 'self'\">\
             <img src={elsewhere}><img src=late.png>"
        )
        .into_boxed_str(),
    );
    let server = gated(page, &gate);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    // Resolved against the base, before the policy in the head; nothing
    // after it.
    let deadline = Instant::now() + WAIT;
    while reached(&server, "/b/based.css") == 0 {
        assert!(Instant::now() < deadline);
        tab.engine.poll_network();
        std::thread::sleep(Duration::from_millis(2));
    }
    idle(&mut tab, 200);
    assert_eq!(reached(&server, "/b/late.png"), 0);
    gate.open();
    finish(&mut tab, &server.url("/page.html"));
    assert_eq!(reached(&server, "/b/based.css"), 1);
    assert_eq!(reached(&server, "/b/late.png"), 1);
    assert!(other.requests().is_empty(), "the policy refused it");
}

#[test]
fn a_request_held_to_a_policy_a_script_gave_since_does_not_take_the_preload() {
    // M38: the preload was made under no policy; a script's `<meta>` then
    // refuses images from elsewhere, so the image's own request is
    // refused, and says so, rather than taking the preload's answer.
    let other = serve(|_| Some(png()));
    let elsewhere = other.url("/x.png").to_string();
    let page: &'static str = Box::leak(
        format!(
            "<script>const m = document.createElement('meta');\
             m.httpEquiv = 'Content-Security-Policy'; m.content = \"img-src 'self'\";\
             document.head.append(m);</script><img src={elsewhere}>"
        )
        .into_boxed_str(),
    );
    let server = serve(move |path| (path == "/page.html").then(|| html(page)));
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    tab.settle();
    let lines: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("Content-Security-Policy refused the image")),
        "{lines:?}"
    );
}

#[test]
fn what_the_cache_holds_fresh_is_not_preloaded() {
    // M38: the page's own request is answered at once from the cache;
    // a preload would only cost. The script is held the second time.
    let gate = Gate::new();
    let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server = {
        let (gate, asked) = (gate.clone(), asked.clone());
        serve(move |path| match path {
            "/page.html" => Some(html("<script src=slow.js></script><img src=kept.png>")),
            "/slow.js" => {
                if asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0 {
                    gate.wait();
                }
                Some(Reply::with_fields(
                    200,
                    &[
                        ("Content-Type", "text/javascript"),
                        ("Cache-Control", "no-store"),
                    ],
                    b"var x = 1;",
                ))
            }
            "/kept.png" => Some(Reply::with_fields(
                200,
                &[
                    ("Content-Type", "image/png"),
                    ("Cache-Control", "max-age=600"),
                ],
                b"",
            )),
            _ => None,
        })
    };
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    assert_eq!(reached(&server, "/kept.png"), 1);
    tab.engine.navigate(server.url("/page.html"));
    let deadline = Instant::now() + WAIT;
    while reached(&server, "/slow.js") < 2 {
        assert!(Instant::now() < deadline);
        tab.engine.poll_network();
        std::thread::sleep(Duration::from_millis(2));
    }
    idle(&mut tab, 100);
    // The script, preloaded, its request taking it; no image.
    assert_eq!(tab.engine.network.preloads_waiting(), 0);
    gate.open();
    finish(&mut tab, &server.url("/page.html"));
    assert_eq!(reached(&server, "/kept.png"), 1, "from the cache");
}
