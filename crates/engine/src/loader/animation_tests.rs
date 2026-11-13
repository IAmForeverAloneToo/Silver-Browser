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

#[test]
fn a_flush_of_an_element_no_frame_styled_is_the_before_change_style_of_the_frames_restyle() {
    // G46: an element a script made and flushed, then changed, is not new
    // to the frame that restyles it after a frame has drawn the page: the
    // flush is its before-change style, and the change transitions.
    let (mut tab, clock, _) = page(
        "<script>\
         requestAnimationFrame(function () {\
           var d = document.createElement('div');\
           d.style.cssText = 'transition: width 5000ms; width: 0px';\
           document.body.appendChild(d);\
           getComputedStyle(d).width;\
           d.style.width = '100px';\
           d.addEventListener('transitionrun', function (e) { console.log(e.type); });\
         });\
         </script>",
    );
    frame_at(&mut tab, &clock, 16);
    frame_at(&mut tab, &clock, 32);
    frame_at(&mut tab, &clock, 48);
    assert_eq!(lines(&mut tab), ["transitionrun"]);
}

#[test]
fn a_flush_before_the_first_frame_is_the_before_change_style_of_its_cascade() {
    // G46, as a test's first task does it: the element made, flushed and
    // changed before any frame styled the page transitions from the
    // flush in the first frame's cascade, once the page has loaded.
    let (mut tab, clock, _) = page(
        "<script src=/none.js></script><body><div id=log></div><script>\
           var d = document.createElement('div');\
           d.style.cssText = 'transition: width 5000ms; width: 0px';\
           document.body.appendChild(d);\
           getComputedStyle(d).width;\
           d.style.width = '100px';\
           d.addEventListener('transitionrun', function (e) { console.log(e.type); });\
         </script>\n</body>\n",
    );
    // Frames drawn only while one is wanted, as a browser draws them.
    let mut at = 0;
    while tab.engine.needs_frame() && at < 160 {
        at += 16;
        frame_at(&mut tab, &clock, at);
    }
    assert_eq!(lines(&mut tab), ["transitionrun"]);
    assert!(tab.engine.needs_frame(), "the transition runs");
}

#[test]
fn a_transition_waiting_out_its_delay_wakes_a_frame_at_its_end() {
    // A frame is wanted when the delay of a transition of no duration
    // ends, by the timeline of a document whose scripts run, which is at
    // its time origin: a caller drawing frames only when one is wanted
    // sees it end.
    let (mut tab, clock, _) = page(
        "<script>         requestAnimationFrame(function () {           var d = document.createElement('div');           d.style.cssText = 'transition: width 0s 300ms; width: 0px';           document.body.appendChild(d);           getComputedStyle(d).width;           d.style.width = '100px';           d.addEventListener('transitionend', function (e) { console.log(e.type); });         });         </script>",
    );
    let mut at = Duration::ZERO;
    while at < Duration::from_secs(1) {
        let next = if tab.engine.needs_frame() {
            Some(at + Duration::from_millis(16))
        } else {
            [tab.engine.next_frame_at(), tab.engine.next_task_at()]
                .into_iter()
                .flatten()
                .min()
        };
        let Some(next) = next else { break };
        at = next.max(at + Duration::from_millis(1));
        frame_at(&mut tab, &clock, at.as_millis() as u64);
    }
    assert_eq!(lines(&mut tab), ["transitionend"]);
}

#[test]
fn a_css_animation_paused_on_a_scroll_timeline_holds_its_start() {
    // Web Animations 2 section 4.4.13: paused from the first, on the
    // root's scroll timeline, it holds the progress of the range's start
    // once ready, and shows it whatever the scroll.
    let (mut tab, clock, _) = page(
        "<style>@keyframes a { from { width: 100px } to { width: 200px } }\
         #f { height: 2000px }</style><div id=f></div><script>\
         var d = document.createElement('div');\
         d.setAttribute('style', 'width: 50px; height: 10px');\
         document.body.prepend(d);\
         d.style.animation = 'a 100s linear paused';\
         d.style.animationTimeline = 'scroll(root)';\
         requestAnimationFrame(function () { requestAnimationFrame(function () {\
           var a = d.getAnimations()[0];\
           a.ready.then(function () {\
             console.log(a.currentTime, getComputedStyle(d).width);\
             document.scrollingElement.scrollTop = 1000;\
             requestAnimationFrame(function () { requestAnimationFrame(function () {\
               console.log(a.currentTime, a.playState, getComputedStyle(d).width);\
             }); });\
           });\
         }); });\
         </script>",
    );
    for at in [16, 32, 48, 64, 80, 96, 112, 128] {
        frame_at(&mut tab, &clock, at);
    }
    assert_eq!(lines(&mut tab), ["0% 100px", "0% paused 100px"]);
}

#[test]
fn a_flush_of_an_element_no_frame_styled_is_its_pseudo_elements_before_change_style() {
    // G46: the flush's styles of the element's `::after` too, which the
    // frame's restyle transitions from.
    let (mut tab, clock, _) = page(
        "<style>.a::after { content: ''; transition: padding-left 1s; padding-left: 1px }\
         .a.on::after { padding-left: 10px }</style><script>\
         requestAnimationFrame(function () {\
           var d = document.createElement('div'); d.className = 'a';\
           document.body.appendChild(d);\
           getComputedStyle(d).paddingLeft;\
           d.classList.add('on');\
           d.addEventListener('transitionrun', function (e) { console.log(e.pseudoElement); });\
         });\
         </script>",
    );
    frame_at(&mut tab, &clock, 16);
    frame_at(&mut tab, &clock, 32);
    frame_at(&mut tab, &clock, 48);
    assert_eq!(lines(&mut tab), ["::after"]);
}

#[test]
fn a_change_of_color_starts_no_transition_of_a_currentcolor_value() {
    // Color 4 section 4.4: `currentcolor` computes to itself in a property
    // other than `color`, so a change of `color` changes no computed value
    // there to transition, though the used colour follows it.
    let (_tab, _clock, console) = page(
        "<div id=test></div><script>\
           var div = document.getElementById('test');\
           var cs = getComputedStyle(div, '');\
           div.style.color = 'red';\
           div.style.backgroundColor = 'currentcolor';\
           cs.backgroundColor;\
           div.style.transition = 'background-color linear 50s -10s';\
           div.style.color = 'blue';\
           console.log(cs.backgroundColor, div.getAnimations().length);\
         </script>",
    );
    assert_eq!(console, ["rgb(0, 0, 255) 0"]);
}

#[test]
fn an_animationend_listener_sees_the_forwards_fill() {
    // The reference browser's listener reads the value the animation
    // fills forwards with, the event's frame having sampled it.
    let (mut tab, clock, _) = page(
        "<style>@keyframes m { to { margin-left: 100px; opacity: 0.5 } }\
         #d { animation: m 0.1s forwards }</style><div id=d></div><script>\
         var d = document.getElementById('d');\
         getComputedStyle(d).opacity;\
         d.addEventListener('animationend', function () {\
           var cs = getComputedStyle(d); console.log(cs.marginLeft, cs.opacity); });\
         </script>",
    );
    for at in [0, 16, 50, 100, 116, 132, 148] {
        frame_at(&mut tab, &clock, at);
    }
    assert_eq!(lines(&mut tab), ["100px 0.5"]);
}

#[test]
fn an_element_first_rendered_transitions_from_its_starting_style() {
    // Transitions 2 section 3.1: the `@starting-style` rules are the
    // before-change style of an element with none, and only that.
    let (mut tab, clock, console) = page(
        "<style>#d { transition: margin-left 100s linear; margin-left: 100px }\
         @starting-style { #d { margin-left: 0px } }</style><script>\
         requestAnimationFrame(function () {\
           var d = document.createElement('div'); d.id = 'd';\
           document.body.appendChild(d);\
           console.log(getComputedStyle(d).marginLeft, d.getAnimations().length);\
           requestAnimationFrame(function () {\
             console.log(document.styleSheets[0].cssRules[1].constructor.name);\
             console.log(document.styleSheets[0].cssRules[1].cssText);\
           });\
         });\
         </script>",
    );
    assert!(console.is_empty());
    frame_at(&mut tab, &clock, 16);
    frame_at(&mut tab, &clock, 32);
    assert_eq!(
        lines(&mut tab),
        [
            "0px 1",
            "CSSStartingStyleRule",
            "@starting-style {\n  #d { margin-left: 0px; }\n}"
        ]
    );
}

#[test]
fn a_style_flushed_then_changed_at_the_next_frame_transitions_from_the_flushed_value() {
    // The pattern sites lean on, kept as a script's first flush starts
    // from the frame's styles (the site sweep's reddit): a value set and
    // read back (no transition yet), then at the next frame a transition
    // put on and the value changed: it runs from the value read.
    let (mut tab, clock, _) = page(
        "<style>#t { width: 10px }</style><div id=t></div><script>\
         var t = document.getElementById('t');\
         requestAnimationFrame(function () {\
           t.style.width = '100px';\
           console.log(getComputedStyle(t).width, t.offsetWidth);\
           requestAnimationFrame(function () {\
             t.style.transition = 'width 10s linear';\
             t.style.width = '200px';\
             console.log(t.getAnimations().length, getComputedStyle(t).width);\
           });\
         });\
         </script>",
    );
    for at in [16, 32, 48, 64] {
        frame_at(&mut tab, &clock, at);
    }
    assert_eq!(lines(&mut tab), ["100px 100", "1 100px"]);
}

#[test]
fn a_class_toggled_with_a_flush_between_transitions() {
    // A class taken off, the style flushed, the class put back: the
    // flush is the before-change style, so the change transitions.
    let (mut tab, clock, _) = page(
        "<style>#t { transition: opacity 10s linear; opacity: 1 } #t.off { opacity: 0 }</style>\
         <div id=t class=off></div><script>\
         var t = document.getElementById('t');\
         requestAnimationFrame(function () {\
           t.classList.remove('off');\
           getComputedStyle(t).opacity;\
           t.classList.add('off');\
           t.offsetWidth;\
           t.classList.remove('off');\
           console.log(t.getAnimations().length, getComputedStyle(t).opacity);\
         });\
         </script>",
    );
    for at in [16, 32, 48] {
        frame_at(&mut tab, &clock, at);
    }
    let said = lines(&mut tab);
    assert_eq!(said.len(), 1, "{said:?}");
    // The change from the flushed 0 back to 1 transitions.
    assert!(said[0].starts_with("1 "), "{said:?}");
}

#[test]
fn a_flush_after_a_transition_ended_sees_its_end_inherited() {
    // The frame's styles hold the transition's value; it ends before
    // the next frame, and a flush then reads its end, inherited down
    // two levels (WPT css-transitions inherit-height-transition).
    let (mut tab, clock, _) = page(
        "<style>div { height: inherit } #outer { height: 50px }\
         #outer.collapsed { height: 0px; transition: height 0.01s }</style>\
         <div id=outer><div><div id=inner>x</div></div></div><script>\
         var outer = document.getElementById('outer');\
         var inner = document.getElementById('inner');\
         requestAnimationFrame(function () {\
           outer.offsetTop;\
           outer.className = 'collapsed';\
         });\
         outer.addEventListener('transitionend', function () {\
           console.log(getComputedStyle(inner).height);\
         });\
         </script>",
    );
    for at in [16, 32, 48, 64, 80] {
        frame_at(&mut tab, &clock, at);
    }
    assert_eq!(lines(&mut tab), ["0px"]);
}
