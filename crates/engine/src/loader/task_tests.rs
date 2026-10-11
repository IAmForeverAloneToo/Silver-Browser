//! The engine's event loop (M26, `docs/design/events.md`): timers run
//! as tasks when the engine's clock says they are due, in their order,
//! each with its microtask checkpoint, clamped as HTML says, for the
//! document shown and for the one on its way.

use std::time::{Duration, Instant};

use net::test_server::{Reply, TestServer};

use super::tests::load_all;
use crate::LoadState;
use crate::testing::*;

/// `body` served, loaded with scripts on and a clock the test moves; the
/// tab, the clock and the server, which must outlive the page's fetches.
fn timed(body: &'static str) -> (Tab, HandClock, TestServer) {
    let server = serve(move |path| match path {
        "/page.html" => Some(html(body)),
        _ => None,
    });
    let mut tab = Tab::new();
    let clock = tab.hand_clock();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    (tab, clock, server)
}

/// The console's lines since they were last taken.
fn lines(tab: &mut Tab) -> Vec<String> {
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

/// Moves the clock on by `ms` and runs the tasks then due; the lines
/// they logged.
fn after(tab: &mut Tab, clock: &HandClock, ms: u64) -> Vec<String> {
    clock.advance(ms);
    tab.engine.run_tasks();
    lines(tab)
}

#[test]
fn timers_run_as_tasks_when_due_in_the_order_they_were_set() {
    let (mut tab, clock, _server) = timed(
        "<script>\
         setTimeout(function () { console.log('a1'); }, 1);\
         setTimeout(function () { console.log('a0'); }, 0);\
         setTimeout(function () { console.log('b0'); });\
         setTimeout(function () { console.log('m-timer'); \
           Promise.resolve().then(function () { console.log('m-micro'); }); }, 2);\
         setTimeout(function () { console.log('m-after'); }, 2);\
         setTimeout(function (x) { console.log('args', x, this === window); }, 5, 'x');\
         setTimeout(\"console.log('string')\", 5);\
         var gone = setTimeout(function () { console.log('cleared'); }, 3);\
         clearTimeout(gone);\
         console.log('script');\
         </script>",
    );
    assert_eq!(lines(&mut tab), ["script"]);
    assert_eq!(tab.engine.next_task_at(), Some(Duration::ZERO));
    assert_eq!(after(&mut tab, &clock, 0), ["a0", "b0"]);
    assert_eq!(tab.engine.next_task_at(), Some(Duration::from_millis(1)));
    assert_eq!(after(&mut tab, &clock, 1), ["a1"]);
    // A microtask queued in a timer runs before the next timer.
    assert_eq!(
        after(&mut tab, &clock, 1),
        ["m-timer", "m-micro", "m-after"]
    );
    // The cleared timer is gone from the host's queue too.
    assert_eq!(tab.engine.next_task_at(), Some(Duration::from_millis(5)));
    assert_eq!(after(&mut tab, &clock, 2), Vec::<String>::new());
    assert_eq!(after(&mut tab, &clock, 1), ["args x true", "string"]);
    assert_eq!(tab.engine.next_task_at(), None);
}

#[test]
fn nested_timers_wait_4ms_past_the_fifth_level_and_intervals_repeat() {
    let (mut tab, clock, _server) = timed(
        "<script>\
         var level = 0;\
         function nest() { level++; console.log('nest ' + level); \
           if (level < 8) setTimeout(nest, 0); }\
         setTimeout(nest, 0);\
         var n = 0;\
         var tick = setInterval(function () { n++; console.log('tick ' + n); \
           if (n === 3) clearInterval(tick); }, 10);\
         </script>",
    );
    // HTML: a timer set by a task nested more than five deep waits 4ms,
    // as in the reference browser.
    assert_eq!(
        after(&mut tab, &clock, 0),
        ["nest 1", "nest 2", "nest 3", "nest 4", "nest 5", "nest 6"]
    );
    assert_eq!(tab.engine.next_task_at(), Some(Duration::from_millis(4)));
    assert_eq!(after(&mut tab, &clock, 4), ["nest 7"]);
    assert_eq!(after(&mut tab, &clock, 4), ["nest 8"]);
    assert_eq!(after(&mut tab, &clock, 2), ["tick 1"]);
    assert_eq!(after(&mut tab, &clock, 10), ["tick 2"]);
    assert_eq!(after(&mut tab, &clock, 10), ["tick 3"]);
    assert_eq!(after(&mut tab, &clock, 10), Vec::<String>::new());
    assert_eq!(tab.engine.next_task_at(), None);
}

#[test]
fn a_timer_that_fails_is_reported_alone_and_its_changes_reach_the_frame() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<p id=p>x</p><script>\
             setTimeout(function () { throw new Error('boom'); });\
             setTimeout(function () { for (;;) {} });\
             setTimeout(function () { document.getElementById('p').textContent = 'changed'; \
               console.log(performance.now()); });\
             console.log(performance.now(), performance.timeOrigin > 1.6e12);\
             </script>",
        )),
        _ => None,
    });
    let mut tab = Tab::new();
    let clock = tab.hand_clock();
    tab.engine.set_scripting(true);
    tab.engine.script_limits = js::Limits {
        steps: 50,
        ..js::Limits::default()
    };
    clock.advance(2);
    load_all(&mut tab, server.url("/page.html"));
    assert_eq!(lines(&mut tab), ["0 true"]);
    tab.engine.frame(200, 100);
    assert!(!tab.engine.needs_frame());
    clock.advance(7);
    assert!(tab.engine.run_tasks());
    let logged = lines(&mut tab);
    let page = server.url("/page.html");
    assert_eq!(
        logged,
        [
            format!("Uncaught Error: boom at {page}:1:36"),
            format!("Script ran too long and was stopped at {page}:1:1"),
            "7".to_owned()
        ]
    );
    let document = tab.engine.document();
    let p = document
        .subtree(document.root())
        .find(|&node| document.attr(node, "id") == Some("p"))
        .expect("the paragraph");
    assert_eq!(document.text_content(p), "changed");
}

#[test]
fn timers_stay_with_their_document() {
    let server = serve(|path| match path {
        "/one.html" => Some(html(
            "<script>setTimeout(function () { console.log('one'); }, 5);</script>",
        )),
        "/two.html" => Some(html("<p>two</p>")),
        _ => None,
    });
    let mut tab = Tab::new();
    let clock = tab.hand_clock();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/one.html"));
    // Its icon's fetch done too: a page left with a fetch in flight is
    // not kept.
    tab.settle();
    load_all(&mut tab, server.url("/two.html"));
    assert_eq!(after(&mut tab, &clock, 10), Vec::<String>::new());
    assert_eq!(tab.engine.next_task_at(), None);
    // Shown again from the back-forward cache, its timer is overdue.
    tab.engine.go_back();
    tab.wait_until(|engine| engine.load_state() == &LoadState::Loaded);
    assert!(tab.engine.next_task_at().is_some());
    assert_eq!(after(&mut tab, &clock, 0), ["one"]);
}

#[test]
fn a_document_on_its_way_runs_its_timers_before_it_is_shown() {
    // The head comes at once, the body 300ms later: the document is not
    // shown until it has a body, and its timer runs meanwhile.
    // The head is long enough to be known as HTML at once.
    let first = format!(
        "<script>setTimeout(function () {{ console.log('timer'); }});</script><!--{}-->",
        "x".repeat(1500)
    );
    let rest = "<body><script>console.log('rest');</script>";
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{first}",
        first.len() + rest.len()
    );
    let server = TestServer::plain(move |_| {
        Reply::Pieces(vec![
            (Duration::ZERO, head.clone().into_bytes()),
            (Duration::from_millis(300), rest.as_bytes().to_vec()),
        ])
    });
    let mut tab = Tab::new();
    let clock = tab.hand_clock();
    tab.engine.set_scripting(true);
    let url = server.url("/page.html");
    tab.engine.navigate(url.clone());
    let deadline = Instant::now() + WAIT;
    while !(tab.engine.url() == Some(&url) && tab.engine.load_state() == &LoadState::Loaded) {
        assert!(Instant::now() < deadline, "the page loads in time");
        tab.engine.poll_network();
        clock.advance(1);
        tab.engine.run_tasks();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(lines(&mut tab), ["timer", "rest"]);
}

#[test]
fn a_document_may_set_no_more_than_its_timers() {
    let (mut tab, _clock, _server) = timed(
        "<script>\
         try { for (var i = 0; i < 100001; i++) setTimeout(function () {}, 1000); } \
         catch (e) { console.log(i, String(e)); }\
         </script>",
    );
    assert_eq!(
        lines(&mut tab),
        ["100000 RangeError: No more than 100000 timers may be set."]
    );
}

#[test]
fn a_message_to_the_window_is_a_task_for_its_target_origin() {
    let (mut tab, clock, _server) = timed(
        "<script>\
         window.onmessage = function (e) { var d = e.data; console.log('message', \
           d instanceof Array ? Array.from(d[0]) + ' ' + d[0].constructor.name + ' ' + \
           Object.is(d[1], -0) : JSON.stringify(d), \
           e.origin === location.origin, e.source === window, e.isTrusted, e.ports.length); \
           if (!window.timed) { window.timed = setTimeout(function () { console.log('timer'); }); } };\
         window.postMessage({ a: [1, 2] }, '*');\
         window.postMessage('own', '/');\
         window.postMessage('other', 'https://other.test');\
         window.postMessage('options', { targetOrigin: location.origin + '/any/path' });\
         window.postMessage('default', {});\
         var u = new Uint8Array([1, 250]);\
         window.postMessage([u, -0], '*', [u.buffer]);\
         console.log('detached', u.byteLength);\
         try { window.postMessage('x', 'nope'); } catch (e) { console.log(e.name, e.message); }\
         console.log('script');\
         </script>",
    );
    assert_eq!(
        lines(&mut tab),
        [
            "detached 0",
            "SyntaxError Failed to execute 'postMessage' on 'Window': \
             Invalid target origin 'nope' in a call to 'postMessage'.",
            "script"
        ]
    );
    assert_eq!(tab.engine.next_task_at(), Some(Duration::ZERO));
    // Each a task of its own, before a timer due when they are (set by
    // the first), the one for another origin dropped.
    assert_eq!(
        after(&mut tab, &clock, 0),
        [
            "message {\"a\":[1,2]} true true true 0",
            "message \"own\" true true true 0",
            "message \"options\" true true true 0",
            "message \"default\" true true true 0",
            "message 1,250 Uint8Array true true true true 0",
            "timer"
        ]
    );
    assert_eq!(tab.engine.next_task_at(), None);
}
