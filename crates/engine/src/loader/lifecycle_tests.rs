//! The document's lifecycle and the rendering steps (M26,
//! `docs/design/events.md`): `readystatechange`, `DOMContentLoaded`,
//! `load` and `pageshow` in the reference browser's order; `pagehide`
//! and `unload` when left; elements' `load` and `error`; animation
//! frames, `resize`, `scroll` and the CSS animation events in a frame;
//! and a hidden window's visibility.

use std::time::Duration;

use net::test_server::Reply;

use super::script_tests::scripted;
use super::tests::load_all;
use crate::testing::*;
use crate::{FrameTime, LoadState};

/// A GIF of one transparent pixel.
const GIF: &[u8] = &[
    0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xff, 0xff, 0xff, 0x21, 0xf9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x44, 0x01, 0x00, 0x3b,
];

/// The head of a response of [`GIF`], its body to follow.
fn gif_head() -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: image/gif\r\nContent-Length: {}\r\n\r\n",
        GIF.len()
    )
    .into_bytes()
}

/// The console's lines since they were last taken.
fn lines(tab: &mut Tab) -> Vec<String> {
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

/// A frame at `ms` on the clock, of 200 by 100.
fn frame_at(tab: &mut Tab, ms: u64) {
    let now = Duration::from_millis(ms);
    tab.engine.frame_at(
        200,
        100,
        FrameTime {
            now,
            deadline: now + Duration::from_millis(16),
        },
    );
}

#[test]
fn the_documents_events_come_in_the_reference_browsers_order() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<script>\
             var log = function (what) { console.log(what + ' ' + document.readyState); };\
             document.addEventListener('readystatechange', function () { log('rsc'); });\
             document.addEventListener('DOMContentLoaded', function (e) { \
               log('DCL ' + e.bubbles + ' ' + e.isTrusted); });\
             window.addEventListener('load', function (e) { log('load ' + \
               (e.target === document) + ' ' + (e.currentTarget === window) + ' ' + \
               e.eventPhase); });\
             window.addEventListener('pageshow', function (e) { log('pageshow ' + \
               e.persisted + ' ' + (e instanceof PageTransitionEvent)); });\
             </script>\
             <script defer src=/defer.js></script><p>x</p>\
             <img src=/slow.gif onload=\"log('img')\">",
        )),
        "/defer.js" => Some(Reply::ok("text/javascript", b"log('defer')")),
        // The image keeps the load waiting after DOMContentLoaded.
        "/slow.gif" => Some(Reply::Pieces(vec![
            (Duration::ZERO, gif_head()),
            (Duration::from_millis(300), GIF.to_vec()),
        ])),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    assert_eq!(
        lines(&mut tab),
        [
            "rsc interactive",
            "defer interactive",
            "DCL true true interactive",
            "img interactive",
            "rsc complete",
            "load true true 2 complete",
            "pageshow false true complete",
        ]
    );
}

#[test]
fn a_bodys_onload_runs_with_no_script_in_the_page() {
    let (_, console) = scripted(
        "<body onload=\"console.log('loaded', event.type, this === window)\">x",
        true,
    );
    assert_eq!(console, ["loaded load true"]);
    let (_, console) = scripted("<body onload=\"console.log('loaded')\">x", false);
    assert!(console.is_empty());
}

#[test]
fn a_page_left_hears_pagehide_and_unload_and_one_shown_again_pageshow() {
    let server = serve(|path| match path {
        "/one.html" => Some(html(
            "<script>\
             addEventListener('pagehide', function (e) { console.log('pagehide', e.persisted); });\
             addEventListener('unload', function () { console.log('unload'); });\
             addEventListener('pageshow', function (e) { console.log('pageshow', e.persisted); });\
             </script>",
        )),
        "/two.html" => Some(html("<p>two</p>")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/one.html"));
    assert_eq!(lines(&mut tab), ["pageshow false"]);
    load_all(&mut tab, server.url("/two.html"));
    // Kept by the back-forward cache: no unload.
    assert_eq!(lines(&mut tab), ["pagehide true"]);
    tab.engine.go_back();
    tab.wait_until(|engine| engine.load_state() == &LoadState::Loaded);
    assert_eq!(lines(&mut tab), ["pageshow true"]);
    // Reloaded, the document is not kept.
    tab.engine.reload();
    tab.wait_until(|engine| engine.load_state() == &LoadState::Loaded);
    assert_eq!(
        lines(&mut tab),
        ["pagehide false", "unload", "pageshow false"]
    );
}

#[test]
fn elements_hear_whether_their_resources_came() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<script>var log = function (e) { console.log(e.type, e.target.id); };</script>\
             <link id=sheet rel=stylesheet href=/a.css onload=log(event) onerror=log(event)>\
             <link id=nosheet rel=stylesheet href=/missing.css onload=log(event) \
               onerror=log(event)>\
             <script id=good src=/a.js onload=log(event) onerror=log(event)></script>\
             <script id=bad src=/missing.js onload=log(event) onerror=log(event)></script>\
             <script id=empty src='' onerror=log(event)></script>\
             <img id=img src=/a.gif onload=log(event) onerror=log(event)>\
             <img id=broken src=/missing.gif onload=log(event) onerror=log(event)>\
             <script>onload = function () { setTimeout(function () { \
               var l = document.createElement('link'); l.id = 'sheet2'; l.rel = 'stylesheet'; \
               l.href = '/a.css?2'; l.onload = log; document.head.appendChild(l); \
               var i = document.createElement('img'); i.id = 'img2'; i.src = '/a.gif'; \
               i.onload = log; document.body.appendChild(i); }); };</script>",
        )),
        // A link hears of its own sheet, not of what that imports.
        "/a.css" | "/a.css?2" => Some(Reply::ok(
            "text/css",
            b"@import url(/b.css); p { color: red }",
        )),
        "/b.css" => Some(Reply::ok("text/css", b"p { color: blue }")),
        "/a.js" => Some(Reply::ok("text/javascript", b"console.log('ran')")),
        "/a.gif" => Some(Reply::ok("image/gif", GIF)),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    let mut seen: Vec<String> = lines(&mut tab)
        .into_iter()
        .filter(|line| !line.starts_with("Failed to load"))
        .collect();
    seen.sort();
    let mut later: Vec<String> = Vec::new();
    let deadline = std::time::Instant::now() + WAIT;
    let mut quiet = None;
    // After the load, a sheet and an image already in: each element
    // hears once, and a sheet's import tells its link nothing.
    while quiet.is_none_or(|since: std::time::Instant| since.elapsed() < Duration::from_millis(300))
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the events come in time"
        );
        tab.engine.poll_network();
        tab.engine.run_tasks();
        later.extend(lines(&mut tab));
        if later.len() >= 2 && quiet.is_none() {
            quiet = Some(std::time::Instant::now());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    later.sort();
    assert_eq!(later, ["load img2", "load sheet2"]);
    assert_eq!(
        seen,
        [
            "error bad",
            "error broken",
            "error empty",
            "error nosheet",
            "load good",
            "load img",
            "load sheet",
            "ran",
        ]
    );
}

#[test]
fn animation_frames_run_before_the_frame_with_its_time() {
    let (mut tab, console) = scripted(
        "<p id=p>x</p><script>\
         var n = 0;\
         function step(t) { n++; console.log('frame', n, t >= 0, this === undefined); \
           if (n < 2) requestAnimationFrame(step); \
           document.getElementById('p').textContent = 'frame ' + n; }\
         requestAnimationFrame(step);\
         var gone = requestAnimationFrame(function () { console.log('cancelled'); });\
         cancelAnimationFrame(gone);\
         requestAnimationFrame(function () { 'use strict'; \
           Promise.resolve().then(function () { console.log('micro'); }); \
           console.log('second', this === undefined); });\
         requestAnimationFrame(function () { console.log('third'); });\
         </script>",
        true,
    );
    assert!(console.is_empty());
    assert!(tab.engine.needs_frame());
    // A frame without a time runs none.
    tab.engine.frame(200, 100);
    assert!(lines(&mut tab).is_empty());
    frame_at(&mut tab, 1000);
    assert_eq!(
        lines(&mut tab),
        ["frame 1 true false", "second true", "micro", "third"]
    );
    assert!(tab.engine.needs_frame());
    frame_at(&mut tab, 1016);
    assert_eq!(lines(&mut tab), ["frame 2 true false"]);
    frame_at(&mut tab, 1032);
    assert!(lines(&mut tab).is_empty());
}

#[test]
fn a_frame_fires_resize_and_scroll_when_they_changed() {
    let (mut tab, _) = scripted(
        "<div id=box style='height: 50px; overflow: auto'><div style='height: 500px'></div></div>\
         <div style='height: 3000px'></div><script>\
         addEventListener('resize', function () { console.log('resize', innerWidth); });\
         document.addEventListener('scroll', function (e) { console.log('scroll document', \
           e.bubbles); });\
         addEventListener('scroll', function (e) { console.log('scroll window', \
           e.target === document); });\
         document.getElementById('box').addEventListener('scroll', function (e) { \
           console.log('scroll box', e.bubbles); });\
         </script>",
        true,
    );
    frame_at(&mut tab, 1000);
    assert!(lines(&mut tab).is_empty());
    tab.engine.frame_at(
        300,
        100,
        FrameTime {
            now: Duration::from_millis(1016),
            deadline: Duration::from_millis(1032),
        },
    );
    assert_eq!(lines(&mut tab), ["resize 300"]);
    tab.engine.scroll_to(100.0);
    let document = tab.engine.document();
    let boxed = document
        .subtree(document.root())
        .find(|&node| document.attr(node, "id") == Some("box"))
        .expect("the box");
    tab.engine.set_element_scroll(boxed, 20.0);
    frame_at(&mut tab, 2000);
    frame_at(&mut tab, 3000);
    // The window went back to 200 wide; the box jumped; the document's
    // scroll is animated, and moved by the next frame.
    assert_eq!(
        lines(&mut tab),
        [
            "resize 200",
            "scroll box false",
            "scroll document true",
            "scroll window true"
        ]
    );
}

#[test]
fn css_animation_events_reach_their_listeners_in_the_next_frame() {
    let (mut tab, _) = scripted(
        "<style>@keyframes fade { from { opacity: 0 } to { opacity: 1 } }\
         #a { animation: fade 100ms }</style><div id=a>x</div><script>\
         var a = document.getElementById('a');\
         ['animationstart', 'animationend'].forEach(function (type) { \
           document.body.addEventListener(type, function (e) { \
             console.log(e.type, e.animationName, e.elapsedTime, e.target === a, \
               e instanceof AnimationEvent); }); });\
         </script>",
        true,
    );
    frame_at(&mut tab, 1000);
    frame_at(&mut tab, 1016);
    assert_eq!(lines(&mut tab), ["animationstart fade 0 true true"]);
    frame_at(&mut tab, 1200);
    // The animation is over, but its event waits for the next frame's
    // rendering steps: that frame is wanted.
    assert!(tab.engine.needs_frame(), "a frame for the queued event");
    frame_at(&mut tab, 1216);
    assert_eq!(lines(&mut tab), ["animationend fade 0.1 true true"]);
    assert!(!tab.engine.needs_frame());
}

#[test]
fn a_hidden_window_hides_its_document_and_slows_its_timers() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<script>\
             document.addEventListener('visibilitychange', function () { \
               console.log('visibility', document.visibilityState, document.hidden); });\
             setInterval(function () { console.log('tick'); }, 10);\
             requestAnimationFrame(function () { console.log('frame'); });\
             </script>",
        )),
        _ => None,
    });
    let mut tab = Tab::new();
    let clock = tab.hand_clock();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    tab.engine.set_hidden(true);
    assert_eq!(lines(&mut tab), ["visibility hidden true"]);
    // No frame is wanted while hidden, though a callback waits, and the
    // interval runs once a second at most.
    tab.engine.frame(200, 100);
    assert!(!tab.engine.needs_frame());
    for _ in 0..200 {
        clock.advance(10);
        tab.engine.run_tasks();
    }
    assert_eq!(lines(&mut tab), ["tick", "tick"]);
    // A frame drawn while hidden runs no callback.
    frame_at(&mut tab, 3000);
    assert!(lines(&mut tab).is_empty());
    tab.engine.set_hidden(false);
    assert_eq!(lines(&mut tab), ["visibility visible false"]);
    assert!(tab.engine.needs_frame());
    frame_at(&mut tab, 3016);
    assert_eq!(lines(&mut tab), ["frame"]);
}

#[test]
fn an_animation_a_frame_callback_finishes_is_heard_before_the_next_callbacks() {
    // css-animations/animation-delay-009: a callback moves a running
    // animation past its end; its start and end reach the listeners in
    // the next frame's rendering steps, before that frame's callbacks.
    let (mut tab, _) = scripted(
        "<style>@keyframes red { from { color: red } to { color: red } }</style>\
         <div>x</div><script>\
         var div = document.querySelector('div');\
         div.style.animation = 'red 1000s';\
         div.addEventListener('animationstart', function () { console.log('start'); });\
         div.addEventListener('animationend', function () { console.log('end'); });\
         requestAnimationFrame(function () { console.log('frame 1');\
           div.style.animationDelay = '-1000s';\
           requestAnimationFrame(function () { console.log('frame 2'); }); });\
         </script>",
        true,
    );
    frame_at(&mut tab, 0);
    assert_eq!(lines(&mut tab), ["frame 1"]);
    frame_at(&mut tab, 16);
    assert_eq!(lines(&mut tab), ["start", "end", "frame 2"]);
}
