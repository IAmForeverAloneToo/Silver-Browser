//! Scroll snapping through a page (CSS Scroll Snap 1 and 2, gap batch
//! H): the snap events scrolls, layouts and styles send, a touch's pan
//! and the scrollbar's thumb, which snap once let go, the keys scrolling
//! the scroll container focused, `scroll-initial-target`, a script's
//! focus scrolled into view, and a script's scroll before the first
//! frame, on a clock the test moves.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use net::test_server::Reply;

use super::tests::load_all;
use crate::testing::*;
use crate::{FrameTime, Key};

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
    tab.engine.set_viewport(800, 600);
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

/// Frames of 800 by 600, `count` of them a frame apart, the clock moved
/// on to each and its tasks run, from where it is.
fn frames(tab: &mut Tab, clock: &AtomicU64, count: u64) {
    for _ in 0..count {
        let at = clock.load(Ordering::Relaxed) + 16;
        clock.store(at, Ordering::Relaxed);
        tab.engine.run_tasks();
        let now = Duration::from_millis(at);
        tab.engine.frame_at(
            800,
            600,
            FrameTime {
                now,
                deadline: now + Duration::from_millis(16),
            },
        );
    }
}

/// Runs `script` in the page, and what the console said.
fn run(tab: &mut Tab, script: &str) -> Vec<String> {
    crate::differential::run_script(&mut tab.engine, script);
    lines(tab)
}

/// A scroll container `#s`, 200px square at the page's top left,
/// snapping down, with 100px areas `#a0`, `#a1` and `#a2` at 0, 300 and
/// 600 and room to 2000, and a script logging its snap events and
/// `scrollend`.
const SNAPPER: &str = "<style>body { margin: 0 } #s { position: relative; width: 200px;
       height: 200px; overflow: scroll; scroll-snap-type: y mandatory }
     .a { position: absolute; left: 0; width: 100px; height: 100px; scroll-snap-align: start }
     #room { height: 2000px }</style>
     <div id=s><div id=room></div><div class=a id=a0 style='top: 0'></div>
     <div class=a id=a1 style='top: 300px'></div><div class=a id=a2 style='top: 600px'></div></div>
     <script>const s = document.getElementById('s');
     const name = (e) => e === null ? 'null' : e.id;
     for (const kind of ['scrollsnapchanging', 'scrollsnapchange']) {
       s.addEventListener(kind, (e) => console.log(kind, name(e.snapTargetBlock),
         name(e.snapTargetInline), e.bubbles, e.cancelable, e instanceof SnapEvent));
     }
     s.addEventListener('scrollend', () => console.log('scrollend', s.scrollTop));</script>";

#[test]
fn a_snap_container_tells_what_it_snaps_to_as_that_changes() {
    // Scroll Snap 2 section 4, as the reference browser sends them: a
    // `scrollsnapchange` for the areas first snapped to; a script's
    // scroll to another `scrollsnapchanging`, then `scrollsnapchange`
    // before its `scrollend`, neither bubbling; none for a scroll that
    // snaps back to the same area; and the areas none once the
    // container stops snapping.
    let (mut tab, clock, _) = page(SNAPPER);
    frames(&mut tab, &clock, 3);
    assert_eq!(
        lines(&mut tab),
        ["scrollsnapchange a0 null false false true"]
    );
    run(&mut tab, "s.scrollTo(0, 280)");
    frames(&mut tab, &clock, 3);
    assert_eq!(
        lines(&mut tab),
        [
            "scrollsnapchanging a1 null false false true",
            "scrollsnapchange a1 null false false true",
            "scrollend 300"
        ]
    );
    run(&mut tab, "s.scrollTo(0, 320)");
    frames(&mut tab, &clock, 3);
    assert!(lines(&mut tab).is_empty());
    // A layout that moves the area followed: no event, the container
    // following it.
    assert_eq!(
        run(
            &mut tab,
            "document.getElementById('a1').style.top = '350px'; console.log(s.scrollTop)"
        ),
        ["350"]
    );
    frames(&mut tab, &clock, 3);
    assert_eq!(lines(&mut tab), ["scrollend 350"]);
    run(&mut tab, "s.style.scrollSnapType = 'none'");
    frames(&mut tab, &clock, 3);
    assert_eq!(
        lines(&mut tab),
        [
            "scrollsnapchanging null null false false true",
            "scrollsnapchange null null false false true"
        ]
    );
    // A script makes no SnapEvent, as the reference browser's cannot.
    assert_eq!(
        run(
            &mut tab,
            "try { new SnapEvent('x', {}); } catch (e) { console.log(e.name, e.message) }"
        ),
        ["TypeError Failed to construct 'SnapEvent': Illegal constructor"]
    );
}

#[test]
fn a_pseudo_elements_area_is_one_of_its_own_named_by_its_element() {
    // The container's `::before` and `::after` are two areas: a scroll
    // from one to the other changes what it is snapped to, though both
    // name the container.
    let (mut tab, clock, _) = page(
        "<style>#s { width: 200px; height: 200px; overflow: scroll; scroll-snap-type: y mandatory }
               #s::before, #s::after { content: ''; display: block; height: 50px;
                 scroll-snap-align: start }
               #s::after { margin-top: 300px }
               #room { height: 1000px }</style>
         <div id=s><div id=room></div></div>
         <script>const s = document.getElementById('s');
           s.addEventListener('scrollsnapchange', (e) => console.log(e.snapTargetBlock.id,
             s.scrollTop));</script>",
    );
    frames(&mut tab, &clock, 3);
    assert_eq!(lines(&mut tab), ["s 0"]);
    run(&mut tab, "s.scrollTo(0, 1300)");
    frames(&mut tab, &clock, 3);
    assert_eq!(lines(&mut tab), ["s 1200"]);
}

#[test]
fn following_the_area_snapped_to_is_at_once_whatever_the_scroll_behavior() {
    // CSS Scroll Snap 1 section 5.2, as the reference browser re-snaps:
    // a container of `scroll-behavior: smooth` follows its area at once,
    // for a script's read and for a frame.
    let (mut tab, clock, _) = page(
        "<style>#s { width: 200px; height: 200px; overflow: scroll; scroll-snap-type: y mandatory;
                 scroll-behavior: smooth; position: relative }
               .a { position: absolute; width: 50px; height: 50px; scroll-snap-align: start }
               #room { height: 2000px }</style>
         <div id=s><div id=room></div><div class=a id=a0 style='top: 0'></div>
           <div class=a style='top: 500px'></div></div>",
    );
    frames(&mut tab, &clock, 3);
    let s = by_id(&tab.engine, "s");
    assert_eq!(
        run(
            &mut tab,
            "document.getElementById('a0').style.top = '50px';
             console.log(document.getElementById('s').scrollTop)"
        ),
        ["50"]
    );
    run(&mut tab, "document.getElementById('a0').style.top = '80px'");
    frames(&mut tab, &clock, 2);
    assert_eq!(tab.engine.element_scroll(s), 80.0);
}

#[test]
fn the_viewports_snap_events_are_the_documents_and_bubble() {
    let (mut tab, clock, _) = page(
        "<style>:root { scroll-snap-type: y mandatory } body { margin: 0 }
               .a { height: 400px; scroll-snap-align: start }</style>
         <div class=a id=a0></div><div class=a id=a1></div><div class=a id=a2></div>
         <script>onscrollsnapchange = (e) => console.log(e.target.nodeName, e.bubbles,
           e.snapTargetBlock.id);</script>",
    );
    frames(&mut tab, &clock, 3);
    assert_eq!(lines(&mut tab), ["#document true a0"]);
    run(&mut tab, "scrollTo(0, 500)");
    frames(&mut tab, &clock, 3);
    assert_eq!(lines(&mut tab), ["#document true a1"]);
}

#[test]
fn a_touch_pans_unsnapped_and_snaps_once_lifted() {
    // The content follows the touch at once, the container told what it
    // would rest on as that changes; lifted, it snaps, the way it went
    // or back to where it was, and its scroll ends.
    let (mut tab, clock, _) = page(SNAPPER);
    frames(&mut tab, &clock, 3);
    lines(&mut tab);
    tab.engine.touch_down(100.0, 190.0);
    tab.engine.touch_move(100.0, 20.0);
    frames(&mut tab, &clock, 2);
    assert_eq!(
        run(&mut tab, "console.log(s.scrollTop)"),
        ["scrollsnapchanging a1 null false false true", "170"]
    );
    tab.engine.touch_up(100.0, 20.0);
    frames(&mut tab, &clock, 3);
    assert_eq!(
        lines(&mut tab),
        ["scrollsnapchange a1 null false false true", "scrollend 300"]
    );
    // A small pan: back to where it was, though the way it went.
    tab.engine.touch_down(100.0, 100.0);
    tab.engine.touch_move(100.0, 90.0);
    tab.engine.touch_up(100.0, 90.0);
    frames(&mut tab, &clock, 3);
    assert_eq!(lines(&mut tab), ["scrollend 300"]);
    // A pan to an area's position, which the snap leaves where it is:
    // its scroll ends all the same.
    tab.engine.touch_down(100.0, 190.0);
    tab.engine.touch_move(100.0, -110.0);
    frames(&mut tab, &clock, 2);
    lines(&mut tab);
    tab.engine.touch_up(100.0, -110.0);
    frames(&mut tab, &clock, 3);
    assert_eq!(
        lines(&mut tab),
        ["scrollsnapchange a2 null false false true", "scrollend 600"]
    );
}

#[test]
fn the_thumb_held_scrolls_the_viewport_unsnapped_until_let_go() {
    let (mut tab, clock, _) = page(
        "<style>:root { scroll-snap-type: y mandatory } body { margin: 0 }
               .a { height: 600px; scroll-snap-align: start }</style>
         <div class=a></div><div class=a></div><div class=a></div>",
    );
    frames(&mut tab, &clock, 2);
    tab.engine.hold_scrollbar(true);
    tab.engine.scroll_to_fraction(0.4);
    frames(&mut tab, &clock, 3);
    assert_eq!(tab.engine.scroll_y(), 480.0);
    tab.engine.hold_scrollbar(false);
    frames(&mut tab, &clock, 3);
    assert_eq!(tab.engine.scroll_y(), 600.0);
}

#[test]
fn the_keys_scroll_the_scroll_container_focused_and_snap_its_way() {
    // A page passes no area it has not shown whole: from 0 the 300px
    // area at 30 is not, though the one at 400 is nearer. An arrow
    // scrolls the container around the focus, not the viewport, where
    // that area covers its snapport, and back.
    let (mut tab, clock, _) = page(
        "<style>body { margin: 0 } #s { position: relative; width: 200px; height: 200px;
                 overflow: auto; scroll-snap-type: y mandatory }
               .a { position: absolute; width: 50px; height: 50px; scroll-snap-align: start }
               #room { height: 2000px }</style>
         <div id=s tabindex=0><div id=room></div><div class=a style='top: 0'></div>
           <div class=a style='top: 30px; height: 300px'></div>
           <div class=a style='top: 400px'></div><div class=a style='top: 900px'></div></div>
         <script>document.getElementById('s').focus();</script>",
    );
    frames(&mut tab, &clock, 2);
    let s = by_id(&tab.engine, "s");
    tab.engine.key_down(&Key::new("PageDown", "PageDown"));
    frames(&mut tab, &clock, 40);
    assert_eq!(tab.engine.element_scroll(s), 30.0);
    assert_eq!(tab.engine.scroll_y(), 0.0);
    tab.engine.key_down(&Key::new("ArrowDown", "ArrowDown"));
    frames(&mut tab, &clock, 40);
    assert_eq!(tab.engine.element_scroll(s), 70.0);
    tab.engine.key_down(&Key::new("ArrowUp", "ArrowUp"));
    frames(&mut tab, &clock, 40);
    assert_eq!(tab.engine.element_scroll(s), 30.0);
}

#[test]
fn scroll_initial_target_places_a_container_until_it_is_scrolled() {
    // Read in the page's own script, before any frame; then kept to as
    // the layout moves the target, until a script scrolls it.
    let (mut tab, clock, said) = page(
        "<style>#s { width: 100px; height: 100px; overflow: scroll }
               #t { height: 50px; scroll-initial-target: nearest; scroll-snap-align: end }</style>
         <div id=s><div id=gap style='height: 300px'></div><div id=t></div>
           <div style='height: 300px'></div></div>
         <script>const s = document.getElementById('s'); console.log(s.scrollTop);</script>",
    );
    assert_eq!(said, ["250"]);
    assert_eq!(
        run(
            &mut tab,
            "document.getElementById('gap').style.height = '200px'; console.log(s.scrollTop)"
        ),
        ["150"]
    );
    run(&mut tab, "s.scrollTop = 20");
    frames(&mut tab, &clock, 2);
    assert_eq!(
        run(
            &mut tab,
            "document.getElementById('gap').style.height = '100px'; console.log(s.scrollTop)"
        ),
        ["20"]
    );
}

#[test]
fn a_scripts_focus_scrolls_the_element_into_view_as_the_reference_browser_does() {
    // Centred where it does not show, past the `scroll-padding`; to the
    // nearer edge where it shows in part; not at all where it shows
    // whole. Focused after its insertion, it keeps the focus through the
    // frames.
    let (mut tab, clock, _) = page(
        "<style>#s { width: 200px; height: 100px; overflow: auto; scroll-padding: 0 50px }
               #w { width: 2000px; height: 10px; position: relative }
               #w div { position: absolute; top: 0; width: 50px; height: 10px }</style>
         <div id=s><div id=w></div></div>",
    );
    frames(&mut tab, &clock, 2);
    assert_eq!(
        run(
            &mut tab,
            "const s = document.getElementById('s');
             document.getElementById('w').innerHTML =
               '<div id=far tabindex=-1 style=\"left: 600px\"></div>' +
               '<div id=part tabindex=-1 style=\"left: 560px\"></div>';
             document.getElementById('far').focus(); console.log(s.scrollLeft);
             document.getElementById('part').focus(); console.log(s.scrollLeft);
             document.getElementById('part').focus(); console.log(s.scrollLeft);"
        ),
        ["525", "510", "510"]
    );
    frames(&mut tab, &clock, 2);
    assert_eq!(
        run(&mut tab, "console.log(document.activeElement.id)"),
        ["part"]
    );
}

#[test]
fn a_scripts_scroll_before_the_first_frame_ends_with_scrollend() {
    // The first frame's `scroll` and `scrollend` tell of it, as they
    // would of one after.
    let (mut tab, clock, _) = page(
        "<style>#s { width: 100px; height: 100px; overflow: scroll }</style>
         <div id=s><div style='height: 500px'></div></div>
         <script>const s = document.getElementById('s');
           s.addEventListener('scroll', () => console.log('scroll', s.scrollTop));
           s.addEventListener('scrollend', () => console.log('scrollend', s.scrollTop));
           s.scrollTop = 120;</script>",
    );
    frames(&mut tab, &clock, 2);
    assert_eq!(lines(&mut tab), ["scroll 120", "scrollend 120"]);
}
