//! The Web Animations API through a page's scripts (D75): animations a
//! script makes and the CSS ones it reaches, their promises and events
//! in the frames, their values in the styles, on a clock the test moves.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use net::test_server::Reply;

use super::tests::load_all;
use crate::FrameTime;
use crate::testing::*;

/// A page of `body` loaded with scripting on, its clock at zero until
/// the test moves it: the tab, the clock's milliseconds, and the
/// console's lines.
fn page(body: &'static str) -> (Tab, Arc<AtomicU64>, Vec<String>) {
    let server = serve(move |path| match path {
        "/page.html" => Some(Reply::ok("text/html", body.as_bytes())),
        _ => None,
    });
    let mut tab = Tab::new();
    let ms = Arc::new(AtomicU64::new(0));
    let hand = Arc::clone(&ms);
    tab.engine.set_clock(Arc::new(move || {
        Duration::from_millis(hand.load(Ordering::Relaxed))
    }));
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    let console = lines(&mut tab);
    drop(server);
    (tab, ms, console)
}

/// The console's lines since they were last taken.
fn lines(tab: &mut Tab) -> Vec<String> {
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

/// The clock moved to `at` milliseconds, its tasks run and a frame
/// drawn then, of 200 by 100.
fn frame_at(tab: &mut Tab, clock: &AtomicU64, at: u64) {
    clock.store(at, Ordering::Relaxed);
    tab.engine.run_tasks();
    let now = Duration::from_millis(at);
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
fn a_scripts_changes_reach_get_computed_style_at_once() {
    let (_tab, _clock, console) = page(
        "<div id=d></div><div id=e></div><script>\
         var d = document.getElementById('d'), e = document.getElementById('e');\
         var a = d.animate({ marginLeft: ['100px', '200px'] }, 100000);\
         var m = function (el) { return getComputedStyle(el).marginLeft; };\
         console.log(m(d));\
         a.cancel(); console.log(m(d));\
         a.currentTime = 50000; console.log(m(d));\
         a.effect.target = e; console.log(m(d), m(e));\
         a.effect.setKeyframes({ marginLeft: ['300px', '300px'] }); console.log(m(e));\
         </script>",
    );
    assert_eq!(console, ["100px", "0px", "150px", "0px 150px", "300px"]);
    // The same after a frame, in the promise's reaction.
    let (mut tab, clock, _) = page(
        "<div id=d></div><script>\
         var d = document.getElementById('d');\
         var a = d.animate({ marginLeft: ['100px', '200px'] }, 100000);\
         var m = function (el) { return getComputedStyle(el).marginLeft; };\
         a.ready.then(function () {\
           a.cancel(); console.log(m(d));\
           a.currentTime = 50000; console.log(m(d));\
         });\
         </script>",
    );
    frame_at(&mut tab, &clock, 100);
    assert_eq!(lines(&mut tab), ["0px", "150px"]);
}

#[test]
fn a_css_transition_is_seeked_cancelled_and_played_again_by_a_script() {
    let (mut tab, clock, console) = page(
        "<div id=d style='margin-left: 0px'></div><script>\
         var d = document.getElementById('d');\
         var m = function () { return getComputedStyle(d).marginLeft; };\
         m();\
         d.style.transition = 'margin-left 100s linear';\
         d.style.marginLeft = '1000px';\
         var t = d.getAnimations()[0];\
         console.log(t.transitionProperty, t.playState, m());\
         t.currentTime = 50000; console.log(m());\
         t.cancel(); console.log(t.playState, m(), d.getAnimations().length);\
         t.play(); console.log(t.playState, m());\
         t.currentTime = 25000; console.log(m());\
         </script>",
    );
    assert_eq!(
        console,
        [
            "margin-left running 0px",
            "500px",
            "idle 1000px 0",
            "running 0px",
            "250px"
        ]
    );
    frame_at(&mut tab, &clock, 100);
    // A delayed transition seeked into its active interval by a script in
    // a later task: started, not cancelled.
    let (mut tab, clock, _) = page(
        "<div id=d style='transition: margin-left 100s 100s'></div><script>\
         var d = document.getElementById('d');\
         getComputedStyle(d).marginLeft; d.style.marginLeft = '100px';\
         var t = d.getAnimations()[0];\
         ['transitionrun', 'transitionstart', 'transitionend', 'transitioncancel'].forEach(\
           function (type) { d.addEventListener(type, function (e) {\
             console.log(e.type, e.elapsedTime);\
           }); });\
         Promise.all([new Promise(function (r) { d.addEventListener('transitionrun', r); }),\
           t.ready]).then(function () { t.currentTime = 100000; });\
         </script>",
    );
    frame_at(&mut tab, &clock, 100);
    frame_at(&mut tab, &clock, 116);
    frame_at(&mut tab, &clock, 132);
    assert_eq!(lines(&mut tab), ["transitionrun 0", "transitionstart 0"]);
}

#[test]
fn a_transition_a_flush_starts_on_a_new_element_outlives_the_frames_restyle() {
    let (mut tab, clock, _) = page(
        "<script>\
         setTimeout(function () {\
           var d = document.createElement('div');\
           d.style.transition = 'margin-left 100s';\
           document.body.appendChild(d);\
           getComputedStyle(d).marginLeft; d.style.marginLeft = '100px';\
           ['transitionrun', 'transitioncancel'].forEach(function (type) {\
             d.addEventListener(type, function (e) { console.log(e.type); }); });\
           console.log(d.getAnimations().length);\
           d.getAnimations()[0].currentTime = 50000;\
         }, 10);\
         </script>",
    );
    frame_at(&mut tab, &clock, 5);
    frame_at(&mut tab, &clock, 20);
    frame_at(&mut tab, &clock, 36);
    frame_at(&mut tab, &clock, 52);
    assert_eq!(lines(&mut tab), ["1", "transitionrun"]);
}

#[test]
fn a_later_tasks_first_flush_starts_the_transitions_its_changes_ask_for() {
    let (mut tab, clock, console) = page(
        "<div id=d style='transition: margin-left 100s'></div><script>\
         var d = document.getElementById('d');\
         getComputedStyle(d).marginLeft;\
         setTimeout(function () {\
           d.style.marginLeft = '100px';\
           console.log(d.getAnimations().length, getComputedStyle(d).marginLeft);\
         }, 10);\
         </script>",
    );
    assert!(console.is_empty());
    frame_at(&mut tab, &clock, 5);
    frame_at(&mut tab, &clock, 20);
    assert_eq!(lines(&mut tab), ["1 0px"]);
}

#[test]
fn a_scripts_flush_is_the_before_change_style_of_the_first_frame() {
    // The script's flush before its change is the style change event the
    // first frame's styles follow: the change transitions.
    let (mut tab, clock, console) = page(
        "<style>div { width: 100px; transition: all 100s, height 0s }\
         .t { width: 200px; height: 50px }</style><div id=d></div><script>\
         var d = document.getElementById('d'), cs = getComputedStyle(d);\
         console.log(cs.width);\
         d.classList.add('t');\
         requestAnimationFrame(function () { console.log(cs.width, cs.height); });\
         </script>",
    );
    assert_eq!(console, ["100px"]);
    frame_at(&mut tab, &clock, 16);
    assert_eq!(lines(&mut tab), ["100px 50px"]);
}

#[test]
fn a_frames_styles_are_the_before_change_style_of_a_scripts_first_flush() {
    // No script flushed before the frame callback's change: the frame's
    // styles are its before-change style, offscreen or not.
    let (mut tab, clock, _) = page(
        "<style>.t { transition: translate 10s linear; translate: 0 }\
         .off { translate: -1000px }</style>\
         <div class=t></div><div class='t off'></div><script>\
         var l = document.querySelectorAll('.t'), a = l[0], b = l[1];\
         requestAnimationFrame(function () {\
           a.style.translate = b.style.translate = '100px';\
           var ta = a.getAnimations()[0], tb = b.getAnimations()[0];\
           console.log(getComputedStyle(a).translate, getComputedStyle(b).translate);\
           Promise.all([ta.ready, tb.ready]).then(function () {\
             console.log(ta.startTime === tb.startTime); });\
         });\
         </script>",
    );
    frame_at(&mut tab, &clock, 16);
    frame_at(&mut tab, &clock, 32);
    assert_eq!(lines(&mut tab), ["0px -1000px", "true"]);
}

#[test]
fn a_filled_css_animation_ends_at_its_end_value() {
    let (mut tab, clock, _) = page(
        "<style>.t { background-color: red } \
         .a { animation: anim 0.1s both } \
         @keyframes anim { from { background-color: red } to { background-color: green } }\
         </style><div id=t class=t></div><script>\
         var t = document.getElementById('t');\
         t.addEventListener('animationend', function () {\
           console.log('end', getComputedStyle(t).backgroundColor); });\
         t.classList.add('a');\
         var link = document.createElement('link'); link.rel = 'stylesheet';\
         link.href = 'data:text/css,'; document.head.appendChild(link);\
         </script>",
    );
    let document = tab.engine.document();
    let t = document
        .subtree(document.root())
        .find(|&n| document.attr(n, "id") == Some("t"))
        .expect("the box");
    for at in (0..20).map(|n| n * 16) {
        frame_at(&mut tab, &clock, at);
    }
    let color = tab.engine.styles().get(t).expect("styled").background_color;
    assert_eq!(lines(&mut tab), ["end rgb(0, 128, 0)"]);
    assert_eq!(color, style::computed::Color::rgb8(0, 128, 0));
}

#[test]
fn commit_styles_writes_the_effect_stack_up_to_the_animation() {
    let (_tab, _clock, console) = page(
        "<div id=d style='opacity: 0.1'></div><script>\
         var d = document.getElementById('d');\
         var a = d.animate({ opacity: [0.2, 0.2] }, { duration: 1000, fill: 'forwards' });\
         var b = d.animate({ opacity: [0.3, 0.3], composite: 'add' }, 1000);\
         a.commitStyles(); console.log(d.getAttribute('style'));\
         b.commitStyles(); console.log(d.getAttribute('style'));\
         a.cancel(); b.cancel(); console.log(getComputedStyle(d).opacity);\
         var c = d.animate({ opacity: 0.7 }, 1); c.finish(); c.commitStyles();\
         console.log(d.getAttribute('style'));\
         </script>",
    );
    // A finished animation commits its end, as if it filled.
    assert_eq!(
        console,
        ["opacity: 0.2;", "opacity: 0.5;", "0.5", "opacity: 0.7;"]
    );
}

#[test]
fn a_seeked_animation_goes_on_with_the_frames() {
    let (mut tab, clock, _) = page(
        "<div id=d></div><script>\
         var a = document.getElementById('d').animate(null, 100000);\
         a.ready.then(function () {\
           a.currentTime = -100000;\
           requestAnimationFrame(function () { requestAnimationFrame(function () {\
             console.log(a.currentTime, a.playState, document.timeline.currentTime); }); });\
         });\
         </script>",
    );
    frame_at(&mut tab, &clock, 100);
    frame_at(&mut tab, &clock, 116);
    frame_at(&mut tab, &clock, 132);
    // The frame callbacks the promise's reaction asked for run in the
    // frame that resolved it, after its update of the animations.
    assert_eq!(lines(&mut tab), ["-99984 running 116"]);
}

#[test]
fn an_animation_a_script_makes_is_ready_at_the_next_frame() {
    let (mut tab, clock, console) = page(
        "<div id=d></div><script>\
         var a = document.getElementById('d').animate({ opacity: [0, 1] }, 1000);\
         console.log(a.playState, a.pending, a.startTime, a.currentTime);\
         a.ready.then(function (r) { console.log('ready', r === a, a.pending, a.startTime); });\
         a.finished.then(function () { console.log('finished', a.currentTime); });\
         a.onfinish = function (e) { console.log('finish', e.currentTime, e.timelineTime); };\
         </script>",
    );
    assert_eq!(console, ["running true null 0"]);
    frame_at(&mut tab, &clock, 100);
    assert_eq!(lines(&mut tab), ["ready true false 100"]);
    frame_at(&mut tab, &clock, 1200);
    frame_at(&mut tab, &clock, 1216);
    assert_eq!(lines(&mut tab), ["finished 1000", "finish 1000 1200"]);
}
