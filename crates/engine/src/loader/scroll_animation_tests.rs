//! Scroll-driven animations through a page (D75): the timelines a script
//! or a style makes, measured at each frame from the layout and the
//! scroll offsets, the animations on them, and their values in the
//! styles, on a clock the test moves.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use net::test_server::Reply;

use super::tests::load_all;
use crate::FrameTime;
use crate::testing::*;

/// A page of `body` loaded with scripting on, its clock at zero until
/// the test moves it, and the console's lines.
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
/// drawn then, of 400 by 300.
fn frame_at(tab: &mut Tab, clock: &AtomicU64, at: u64) {
    clock.store(at, Ordering::Relaxed);
    tab.engine.run_tasks();
    let now = Duration::from_millis(at);
    tab.engine.frame_at(
        400,
        300,
        FrameTime {
            now,
            deadline: now + Duration::from_millis(16),
        },
    );
}

/// A scroll container 100px high of seven boxes 50px high, the fourth
/// `#target`, whose view timeline runs its animation; at each frame the
/// page logs the animation's value, then scrolls to the next of `tops`.
const VIEW: &str = "<style>@keyframes anim { from { z-index: 0 } to { z-index: 100 } }\
     #s { overflow: auto; width: 100px; height: 100px }\
     #s > div { height: 50px; z-index: -1; position: relative }\
     #target { view-timeline: --t; animation: anim 1s linear forwards; animation-timeline: --t }\
     </style><div id=s><div></div><div></div><div></div><div id=target></div>\
     <div></div><div></div><div></div></div><script>\
     var s = document.getElementById('s'), t = document.getElementById('target');\
     var tops = [0, 25, 50, 125, 200, 201], i = 0;\
     function log() { console.log(getComputedStyle(t).zIndex); }\
     log();\
     function step() {\
       log();\
       if (i < tops.length) { s.scrollTop = tops[i++]; requestAnimationFrame(step); }\
       else { document.getAnimations()[0].effect.updateTiming({ fill: 'none' }); log(); }\
     }\
     requestAnimationFrame(step);\
     </script>";

#[test]
fn a_view_timeline_runs_its_animation_as_its_subject_crosses_the_scrollport() {
    let (mut tab, clock, console) = page(VIEW);
    // No frame has measured the timeline: the animation shows nothing.
    assert_eq!(console, ["-1"]);
    for frame in 1..=8 {
        frame_at(&mut tab, &clock, frame * 16);
    }
    // Each frame shows the scroll the one before it made: the subject,
    // from 150px to 200px, enters the 100px scrollport at 50px and has
    // left it at 200px, its fill holding the end past it.
    assert_eq!(
        lines(&mut tab),
        ["-1", "-1", "-1", "0", "50", "100", "100", "-1"]
    );
}

/// A scroll container 100px high with 1000px of content.
const SCROLLER: &str = "<div id=s style='overflow: hidden; width: 100px; height: 100px'>\
     <div style='height: 1000px'></div></div>";

#[test]
fn a_scroll_methods_promise_settles_where_the_scroll_ends_or_is_cut_short() {
    let (mut tab, clock, console) = page(Box::leak(
        format!(
            "{SCROLLER}<script>requestAnimationFrame(function () {{\
             var s = document.getElementById('s');\
             var first =s.scrollTo({{ top: 300, behavior: 'smooth' }});\
             s.scrollTop = 50;\
             first.then(function (r) {{ console.log('first', r.interrupted); }});\
             s.scrollTo({{ top: 200 }}).then(function (r) {{\
               console.log('second', r.interrupted, s.scrollTop); }});\
             requestAnimationFrame(function () {{\
               s.scrollTo({{ top: 400, behavior: 'smooth' }}).then(function (r) {{\
                 console.log('third', r.interrupted, s.scrollTop); }});\
             }});\
             }});</script>"
        )
        .into_boxed_str(),
    ));
    assert!(console.is_empty());
    frame_at(&mut tab, &clock, 16);
    // The smooth scroll a later one cut short settles interrupted; an
    // instant one at once.
    assert_eq!(lines(&mut tab), ["first true", "second false 200"]);
    for frame in 2..=120 {
        frame_at(&mut tab, &clock, frame * 16);
    }
    // A smooth one once it is where it was going.
    assert_eq!(lines(&mut tab), ["third false 400"]);
}

#[test]
fn a_scroll_timeline_a_script_makes_has_its_time_at_once_and_drives_its_animation() {
    let (mut tab, clock, console) = page(Box::leak(
        format!(
            "{SCROLLER}<script>requestAnimationFrame(function () {{\
             var s = document.getElementById('s');\
             s.scrollTop = 450;\
             var t = new ScrollTimeline({{ source: s }});\
             console.log(t.currentTime.unit, Math.round(t.currentTime.value), t.duration + '');\
             var a = s.animate({{ opacity: [0, 1] }}, {{ timeline: t }});\
             a.ready.then(function () {{\
               console.log(a.startTime + '', Math.round(a.currentTime.value),\
                 a.effect.getComputedTiming().duration + '');\
             }});\
             }});</script>"
        )
        .into_boxed_str(),
    ));
    assert!(console.is_empty());
    frame_at(&mut tab, &clock, 16);
    // Half of its 900px range, the duration 100%.
    assert_eq!(lines(&mut tab), ["percent 50 100%"]);
    frame_at(&mut tab, &clock, 32);
    // Aligned with the start of its range, its auto duration all of it.
    assert_eq!(lines(&mut tab), ["0% 50 100%"]);
}
