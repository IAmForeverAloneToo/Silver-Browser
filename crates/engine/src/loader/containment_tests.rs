//! Gap batch H's gaps found while closing gaps, through a page's
//! scripts and frames: caret positions from points, `checkVisibility()`,
//! `content-visibility` and its relevance to the user, the custom
//! highlights and what they paint, `pointer-events`, and the focus
//! order of reading flows and of scrollers.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use net::test_server::Reply;

use super::tests::load_all;
use crate::FrameTime;
use crate::testing::*;

/// A page of `body` loaded with scripting on in a viewport of 800 by
/// 600, its clock at zero until the test moves it: the tab, the clock's
/// milliseconds, and the console's lines.
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

/// The clock moved to `at` milliseconds, its tasks run and a frame of
/// 800 by 600 drawn then.
fn frame_at(tab: &mut Tab, clock: &AtomicU64, at: u64) -> Vec<u8> {
    clock.store(at, Ordering::Relaxed);
    tab.engine.run_tasks();
    let now = Duration::from_millis(at);
    tab.engine
        .frame_at(
            800,
            600,
            FrameTime {
                now,
                deadline: now + Duration::from_millis(16),
            },
        )
        .to_vec()
}

/// The pixel at `(x, y)` of an 800-wide frame's RGBA bytes.
fn pixel(frame: &[u8], x: usize, y: usize) -> [u8; 4] {
    let at = (y * 800 + x) * 4;
    [frame[at], frame[at + 1], frame[at + 2], frame[at + 3]]
}

#[test]
fn a_caret_position_is_the_boundary_nearest_the_point() {
    let (_tab, _clock, console) = page(
        "<style>body { margin: 0; font: 20px monospace } div { width: 400px }</style>\
         <div id=t>abcdef</div><div id=e style='height: 50px'></div>\
         <canvas id=c width=40 height=20 style='display: block'></canvas>\
         <div><img id=i style='width: 40px; height: 20px; display: inline-block'></div>\
         <input id=f value='hello' style='display: block; font: 20px monospace'>\
         <script>\
         var t = document.getElementById('t'), r = t.getBoundingClientRect();\
         var w = r.width;\
         var p = document.caretPositionFromPoint(2, r.top + 5);\
         console.log(p instanceof CaretPosition, p.offsetNode === t.firstChild, p.offset);\
         function edge(n) { var b = document.createRange(); b.setStart(t.firstChild, n);\
           return b.getBoundingClientRect().x; }\
         var two = edge(2), three = edge(3);\
         p = document.caretPositionFromPoint(two + (three - two) * 0.4, r.top + 5);\
         console.log(p.offset, p.getClientRect().width, p.getClientRect().x === two,\
           p.getClientRect().height === document.createRange().getBoundingClientRect().height ||\
           p.getClientRect().height > 0);\
         p = document.caretPositionFromPoint(two + (three - two) * 0.6, r.top + 5);\
         console.log(p.offset);\
         var ch = (edge(6) - edge(0)) / 6;\
         p = document.caretPositionFromPoint(390, r.top + 5);\
         console.log(p.offset);\
         var e = document.getElementById('e').getBoundingClientRect();\
         p = document.caretPositionFromPoint(5, e.top + 5);\
         console.log(p.offsetNode.id, p.offset);\
         var c = document.getElementById('c').getBoundingClientRect();\
         p = document.caretPositionFromPoint(5, c.top + 5);\
         console.log(p.offsetNode.id, p.offset);\
         var i = document.getElementById('i'), ir = i.getBoundingClientRect();\
         p = document.caretPositionFromPoint(ir.left + 5, ir.top + 5);\
         console.log(p.offsetNode === i.parentNode, p.offset);\
         p = document.caretPositionFromPoint(ir.left + 35, ir.top + 5);\
         console.log(p.offset);\
         var f = document.getElementById('f'), fr = f.getBoundingClientRect();\
         p = document.caretPositionFromPoint(fr.left + 2 + ch * 3.1, fr.top + fr.height / 2);\
         console.log(p.offsetNode.id, p.offset);\
         var range = document.caretRangeFromPoint(fr.left + 2 + ch * 3.1, fr.top + fr.height / 2);\
         console.log(range.startContainer === f.parentNode, range.collapsed);\
         console.log(document.caretPositionFromPoint(-1, 5), document.caretPositionFromPoint(5, 900));\
         try { document.caretPositionFromPoint('x', 5); } catch (x) { console.log(x.name); }\
         try { document.caretPositionFromPoint(5); } catch (x) { console.log(x.name); }\
         try { document.caretPositionFromPoint(5, 5, { shadowRoots: [document.body] }); }\
         catch (x) { console.log(x.name); }\
         console.log(document.caretRangeFromPoint().startOffset);\
         try { new CaretPosition(); } catch (x) { console.log(x.name); }\
         </script>",
    );
    assert_eq!(
        console,
        [
            "true true 0",
            "2 0 true true",
            "3",
            "6",
            "e 0",
            "c 0",
            "true 0",
            "1",
            "f 3",
            "true true",
            "null null",
            "TypeError",
            "TypeError",
            "TypeError",
            "0",
            "TypeError",
        ]
    );
}

#[test]
fn check_visibility_reads_boxes_opacity_visibility_and_skipped_contents() {
    let (mut tab, clock, console) = page(
        "<style>body { margin: 0 }</style>\
         <div id=n style='display: none'><p id=nn>x</p></div>\
         <div id=c style='display: contents'><p id=cc>x</p></div>\
         <div id=o style='opacity: 0'><p id=oo>x</p></div>\
         <div id=v style='visibility: hidden'>x</div>\
         <div id=h style='content-visibility: hidden'><p id=hh>x</p></div>\
         <div id=a style='content-visibility: auto'><p id=aa>x</p></div>\
         <script>\
         function cv(id, o) { return document.getElementById(id).checkVisibility(o); }\
         console.log(cv('n'), cv('nn'), cv('c'), cv('cc'));\
         console.log(cv('oo'), cv('oo', { checkOpacity: true }), cv('o', { opacityProperty: true }));\
         console.log(cv('v'), cv('v', { checkVisibilityCSS: true }), cv('v', { visibilityProperty: true }));\
         console.log(cv('h'), cv('hh'), cv('aa'), cv('aa', { contentVisibilityAuto: true }));\
         requestAnimationFrame(function () { requestAnimationFrame(function () {\
           requestAnimationFrame(function () {\
             console.log(cv('aa', { contentVisibilityAuto: true }));\
           });\
         }); });\
         </script>",
    );
    assert_eq!(
        console,
        [
            "false false false true",
            "true false false",
            "true false false",
            "true false true false",
        ]
    );
    for at in [16, 32, 48, 64] {
        frame_at(&mut tab, &clock, at);
    }
    assert_eq!(lines(&mut tab), ["true"]);
}

#[test]
fn an_auto_element_is_skipped_until_found_near_and_says_so() {
    let (mut tab, clock, console) = page(
        "<style>body { margin: 0 } .a { content-visibility: auto; \
         contain-intrinsic-size: auto 100px } .big { height: 300px }</style>\
         <div id=on class=a><div class=big>on</div></div>\
         <div style='height: 3000px'></div>\
         <div id=off class=a><div class=big>off</div></div>\
         <script>\
         ['on', 'off'].forEach(function (id) {\
           document.getElementById(id).addEventListener('contentvisibilityautostatechange',\
             function (e) { console.log(id, e.skipped, e.bubbles,\
               e instanceof ContentVisibilityAutoStateChangeEvent); });\
         });\
         function h(id) { return document.getElementById(id).getBoundingClientRect().height; }\
         console.log(h('on'), h('off'));\
         </script>",
    );
    assert_eq!(console, ["100 100"]);
    // Found at once as the first frame lays them out: the one near the
    // viewport relevant, the other skipped.
    frame_at(&mut tab, &clock, 16);
    assert_eq!(
        lines(&mut tab),
        ["on false true true", "off true true true"]
    );
    frame_at(&mut tab, &clock, 32);
    assert!(lines(&mut tab).is_empty());
    // Laid out by its content now, and its size remembered.
    tab.engine
        .run_classic("console.log(document.getElementById('on').getBoundingClientRect().height)");
    assert_eq!(lines(&mut tab), ["300"]);
    frame_at(&mut tab, &clock, 48);
    // Scrolled away, it skips again, at the size it had.
    tab.engine.run_classic("scrollTo(0, 3000)");
    for at in [64, 80, 96] {
        frame_at(&mut tab, &clock, at);
    }
    let said = lines(&mut tab);
    assert!(said.contains(&"on true true true".to_owned()), "{said:?}");
    assert!(said.contains(&"off false true true".to_owned()), "{said:?}");
    tab.engine
        .run_classic("console.log(document.getElementById('on').getBoundingClientRect().height)");
    assert_eq!(lines(&mut tab), ["300"]);
}

#[test]
fn the_focus_or_the_selection_in_an_auto_element_makes_it_relevant() {
    let (mut tab, clock, _) = page(
        "<style>body { margin: 0 } .a { content-visibility: auto }</style>\
         <div style='height: 3000px'></div>\
         <div id=f class=a><button id=b>b</button></div>\
         <div id=s class=a><span id=t>text</span></div>\
         <div id=n class=a>none</div>\
         <script>\
         ['f', 's', 'n'].forEach(function (id) {\
           document.getElementById(id).addEventListener('contentvisibilityautostatechange',\
             function (e) { console.log(id, e.skipped); });\
         });\
         </script>",
    );
    frame_at(&mut tab, &clock, 16);
    assert_eq!(lines(&mut tab), ["f true", "s true", "n true"]);
    // Focusable though skipped, as in the reference browser.
    tab.engine.run_classic(
        "document.getElementById('b').focus({preventScroll: true});\
         getSelection().selectAllChildren(document.getElementById('t'));\
         console.log(document.activeElement.id);",
    );
    assert_eq!(lines(&mut tab), ["b"]);
    frame_at(&mut tab, &clock, 32);
    assert_eq!(lines(&mut tab), ["f false", "s false"]);
    tab.engine
        .run_classic("document.activeElement.blur(); getSelection().removeAllRanges();");
    frame_at(&mut tab, &clock, 48);
    assert_eq!(lines(&mut tab), ["f true", "s true"]);
    // A `hidden` element's contents take no focus.
    tab.engine.run_classic(
        "var h = document.createElement('div'); h.style.contentVisibility = 'hidden';\
         h.innerHTML = '<button id=x>x</button>'; document.body.append(h);\
         document.getElementById('x').focus(); console.log(document.activeElement.id,\
         h.innerText === '');",
    );
    assert_eq!(lines(&mut tab), [" true"]);
}

#[test]
fn skipped_contents_are_not_painted_nor_hit() {
    let (mut tab, clock, console) = page(
        "<style>body { margin: 0 } #h { content-visibility: hidden; width: 100px; \
         contain-intrinsic-size: 100px 50px; background: rgb(0, 0, 255) } \
         #in { width: 80px; height: 30px; background: rgb(255, 0, 0) }</style>\
         <div id=h><div id=in></div></div>\
         <script>\
         var h = document.getElementById('h');\
         console.log(h.getBoundingClientRect().height, document.elementFromPoint(10, 10).id,\
           document.getElementById('in').getBoundingClientRect().height);\
         </script>",
    );
    assert_eq!(console, ["50 h 30"]);
    let frame = frame_at(&mut tab, &clock, 16);
    assert_eq!(pixel(&frame, 10, 10), [0, 0, 255, 255]);
}

#[test]
fn paint_containment_clips_the_contents_to_the_padding_box() {
    let (mut tab, clock, console) = page(
        "<style>body { margin: 0 } #p { contain: paint; width: 50px; height: 50px } \
         #in { width: 100px; height: 100px; background: rgb(255, 0, 0) } \
         #v { width: 50px; height: 50px } \
         #vin { width: 100px; height: 100px; background: rgb(0, 0, 255) }</style>\
         <div id=p><div id=in></div></div><div id=v><div id=vin></div></div>\
         <script>console.log(document.elementFromPoint(75, 25).tagName);</script>",
    );
    // Not hit past the clip either.
    assert_eq!(console, ["BODY"]);
    let frame = frame_at(&mut tab, &clock, 16);
    assert_eq!(pixel(&frame, 25, 25), [255, 0, 0, 255]);
    assert_eq!(pixel(&frame, 75, 25), [255, 255, 255, 255]);
    // Without containment, the content overflows.
    assert_eq!(pixel(&frame, 75, 75), [0, 0, 255, 255]);
}

#[test]
fn pointer_events_none_is_passed_over_by_the_hit_test() {
    let (_tab, _clock, console) = page(
        "<style>body { margin: 0 } div { position: absolute; left: 0; top: 0; \
         width: 100px; height: 100px }</style>\
         <div id=under></div><div id=over style='pointer-events: none'>\
         <span id=again style='pointer-events: auto; display: block; height: 10px'></span></div>\
         <script>\
         console.log(document.elementFromPoint(50, 50).id, document.elementFromPoint(5, 5).id,\
           getComputedStyle(document.getElementById('over')).pointerEvents);\
         </script>",
    );
    assert_eq!(console, ["under again none"]);
}

#[test]
fn a_registered_highlight_paints_its_ranges_text() {
    let (mut tab, clock, console) = page(
        "<style>body { margin: 0; font: 40px monospace } \
         ::highlight(mark) { background-color: rgb(255, 255, 0); color: rgb(0, 128, 0) }</style>\
         <div id=t>aaaaaaaa</div>\
         <script>\
         var t = document.getElementById('t').firstChild;\
         var r = new Range(); r.setStart(t, 2); r.setEnd(t, 4);\
         var h = new Highlight(r);\
         CSS.highlights.set('mark', h);\
         console.log(CSS.highlights.size, h.size, h.priority, h.type, CSS.highlights.get('mark') === h);\
         console.log(getComputedStyle(document.getElementById('t'), '::highlight(mark)').backgroundColor);\
         var x = document.createRange(); x.setStart(t, 0); x.setEnd(t, 1);\
         console.log(Math.round(x.getBoundingClientRect().width));\
         </script>",
    );
    assert_eq!(console[0], "1 1 0 highlight true");
    assert_eq!(console[1], "rgb(255, 255, 0)");
    let width: usize = console[2].parse().expect("a width");
    let frame = frame_at(&mut tab, &clock, 16);
    // The third character's cell is yellow under the text, the first's
    // white.
    let yellow = (0..width)
        .flat_map(|dx| (2..40).map(move |y| (2 * width + dx, y)))
        .filter(|&(x, y)| pixel(&frame, x, y) == [255, 255, 0, 255])
        .count();
    assert!(yellow > width * 10, "{yellow}");
    let white = (0..width)
        .flat_map(|dx| (2..40).map(move |y| (dx, y)))
        .filter(|&(x, y)| pixel(&frame, x, y) == [255, 255, 0, 255])
        .count();
    assert_eq!(white, 0);
    // Taken out of the registry, it paints nothing.
    tab.engine.run_classic("CSS.highlights.delete('mark')");
    let frame = frame_at(&mut tab, &clock, 32);
    let yellow = (0..width)
        .flat_map(|dx| (2..40).map(move |y| (2 * width + dx, y)))
        .filter(|&(x, y)| pixel(&frame, x, y) == [255, 255, 0, 255])
        .count();
    assert_eq!(yellow, 0);
}

#[test]
fn the_higher_priority_paints_on_top_whatever_the_order_registered() {
    let (mut tab, clock, console) = page(
        "<style>body { margin: 0; font: 40px monospace } \
         ::highlight(top) { background-color: rgb(255, 0, 0) } \
         ::highlight(low) { background-color: rgb(255, 255, 0) }</style>\
         <div id=t>aaaa</div>\
         <script>\
         var t = document.getElementById('t').firstChild;\
         var r = new Range(); r.setStart(t, 0); r.setEnd(t, 4);\
         var top = new Highlight(r); top.priority = 1;\
         CSS.highlights.set('top', top); CSS.highlights.set('low', new Highlight(r));\
         var x = document.createRange(); x.setStart(t, 0); x.setEnd(t, 1);\
         console.log(Math.round(x.getBoundingClientRect().width));\
         </script>",
    );
    let width: usize = console[0].parse().expect("a width");
    let frame = frame_at(&mut tab, &clock, 16);
    let red = (0..width)
        .flat_map(|dx| (2..40).map(move |y| (dx, y)))
        .filter(|&(x, y)| pixel(&frame, x, y) == [255, 0, 0, 255])
        .count();
    assert!(red > width * 10, "{red}");
}

#[test]
fn highlights_are_set_like_and_the_registry_map_like() {
    let (_tab, _clock, console) = page(
        "<span id=p>0123456789</span><script>\
         var r1 = new Range(), r2 = new Range();\
         var s = new StaticRange({ startContainer: document.body, startOffset: 0,\
           endContainer: document.body, endOffset: 1 });\
         var h = new Highlight(r1, r1, s);\
         console.log(h.size, h.has(r1), h.has(r2), Highlight.length);\
         h.add(r2); h.delete(r1);\
         var seen = []; h.forEach(function (v, k, set) { seen.push(v === k && set === h); });\
         console.log(seen.join(), [...h].length, h[Symbol.iterator] === h.values);\
         h.priority = 2.7; h.type = 'spelling-error'; h.type = 'bogus';\
         console.log(h.priority, h.type);\
         try { h.add({}); } catch (x) { console.log(x.name); }\
         try { CSS.highlights.set('x', {}); } catch (x) { console.log(x.name); }\
         try { new HighlightRegistry(); } catch (x) { console.log(x.name); }\
         CSS.highlights.set('b', h).set('a', new Highlight());\
         console.log([...CSS.highlights.keys()].join(), CSS.highlights.has('a'),\
           String(CSS.highlights), String(h));\
         var t = document.getElementById('p').firstChild;\
         var a = new Range(); a.setStart(t, 2); a.setEnd(t, 10);\
         var b = new Range(); b.setStart(t, 5); b.setEnd(t, 10);\
         CSS.highlights.clear();\
         var ha = new Highlight(a), hb = new Highlight(b);\
         CSS.highlights.set('one', ha); CSS.highlights.set('two', hb);\
         var box = document.getElementById('p').getBoundingClientRect();\
         function edge(n) { var e = document.createRange(); e.setStart(t, n);\
           return e.getBoundingClientRect().x; }\
         function at(n) { return CSS.highlights.highlightsFromPoint((edge(n) + edge(n + 1)) / 2,\
           box.top + box.height / 2); }\
         console.log(at(0).length, at(3).length, at(3)[0].highlight === ha,\
           at(7).length, at(7)[0].highlight === hb, at(7)[0].ranges[0] === b);\
         hb.priority = -1;\
         console.log(at(7)[0].highlight === ha);\
         </script>",
    );
    assert_eq!(
        console,
        [
            "2 true false 0",
            "true,true 2 true",
            "2 spelling-error",
            "TypeError",
            "TypeError",
            "TypeError",
            "b,a true [object HighlightRegistry] [object Highlight]",
            "0 1 true 2 true true",
            "true",
        ]
    );
}

#[test]
fn get_computed_style_takes_a_highlight_and_refuses_what_does_not_parse() {
    let (_tab, _clock, console) = page(
        "<style>#p::highlight(name) { color: rgb(0, 128, 0) }</style><p id=p>x</p><script>\
         var p = document.getElementById('p');\
         ['::highlight(name)', '::highlight( n\\\\61me ', '::highlight(name)a', ':highlight(name)',\
          '::highlight()', '::before(x)', '::before', ':before', 'before'].forEach(function (s) {\
           var c = getComputedStyle(p, s); console.log(s, c.length > 0, c.color);\
         });\
         </script>",
    );
    assert_eq!(
        console,
        [
            "::highlight(name) true rgb(0, 128, 0)",
            "::highlight( n\\61me  true rgb(0, 128, 0)",
            "::highlight(name)a false ",
            ":highlight(name) false ",
            "::highlight() false ",
            "::before(x) false ",
            "::before true rgb(0, 0, 0)",
            ":before true rgb(0, 0, 0)",
            "before true rgb(0, 0, 0)",
        ]
    );
}

/// The IDs of what Tab focuses in turn from nothing, until the page
/// lets the focus go.
fn tab_order(tab: &mut Tab) -> Vec<String> {
    let mut order = Vec::new();
    while tab.engine.focus_next(false) {
        let document = tab.engine.document();
        let id = tab
            .engine
            .focused()
            .and_then(|node| document.attr(node, "id"))
            .unwrap_or("?")
            .to_owned();
        order.push(id);
        assert!(order.len() < 40, "{order:?}");
    }
    order
}

#[test]
fn tab_reaches_a_scroller_with_nothing_focusable_in_it() {
    let (mut tab, clock, console) = page(
        "<style>.s { overflow: auto; width: 100px; height: 50px } .long { height: 200px }\
         </style>\
         <button id=start>s</button>\
         <div id=t0 class=s tabindex=0><button id=a>a</button><div class=long></div></div>\
         <div id=n1 class=s><button id=b>b</button><div class=long></div></div>\
         <div id=n2 class=s><div class=long></div></div>\
         <div id=neg class=s tabindex=-1><div class=long></div></div>\
         <div id=short class=s><div>short</div></div>\
         <div id=hidden class=s style='overflow: hidden'><div class=long></div></div>\
         <button id=end>e</button>\
         <script>\
         var n2 = document.getElementById('n2'), short = document.getElementById('short');\
         n2.focus(); console.log(document.activeElement.id, n2.tabIndex);\
         short.focus(); console.log(document.activeElement.id);\
         document.getElementById('n1').focus(); console.log(document.activeElement.id);\
         n2.style.height = '400px'; n2.focus(); console.log(document.activeElement.id);\
         n2.style.height = '';\
         </script>",
    );
    assert_eq!(console, ["n2 -1", "n2", "n1", "n1"]);
    frame_at(&mut tab, &clock, 16);
    tab.engine.focus(None, false, false);
    assert_eq!(tab_order(&mut tab), ["start", "t0", "a", "b", "n2", "end"]);
}

#[test]
fn a_reading_flow_orders_its_items_for_tab() {
    let (mut tab, clock, _) = page(
        "<style>.f { display: flex; width: 400px } .g { display: grid; \
         grid-template-columns: 50px 50px } button { width: 40px; height: 20px }</style>\
         <div class=f style='reading-flow: flex-visual; flex-direction: row-reverse'>\
           <button id=a1>a</button><button id=b1>b</button><button id=c1>c</button></div>\
         <div class=f style='reading-flow: flex-flow'>\
           <button id=a2>a</button><button id=b2 style='order: -1'>b</button>\
           <button id=c2 style='reading-order: -1'>c</button></div>\
         <div class=g style='reading-flow: grid-rows'>\
           <button id=a3 style='grid-row: 2'>a</button><button id=b3>b</button>\
           <button id=c3>c</button></div>\
         <div class=g style='reading-flow: grid-columns'>\
           <button id=a4>a</button><button id=b4>b</button><button id=c4>c</button></div>\
         <div class=g style='reading-flow: grid-order'>\
           <div id=h tabindex=0 style='order: -1'>h<span id=h1 tabindex=0>1</span>\
           <span id=h2 tabindex=1>2</span></div>\
           <button id=d5 tabindex=1>d</button></div>\
         <div style='reading-flow: source-order'>\
           <button id=a6 style='reading-order: 1'>a</button><button id=b6>b</button></div>\
         <div style='reading-flow: grid-rows'><button id=a7>a</button><button id=b7>b</button></div>\
         <button id=last tabindex=2>z</button>",
    );
    frame_at(&mut tab, &clock, 16);
    assert_eq!(
        tab_order(&mut tab),
        [
            // A positive tabindex outside every reading flow first.
            "last", "c1", "b1", "a1", "c2", "b2", "a2", "b3", "c3", "a3", "a4", "c4", "b4", "h",
            "h2", "h1", "d5", "b6", "a6", "a7", "b7",
        ]
    );
}

#[test]
fn a_reading_flow_groups_contents_mixes_slots_and_ends_with_what_is_out_of_flow() {
    let (mut tab, clock, _) = page(
        "<style>.g { display: grid; reading-flow: grid-order } button { width: 40px } \
         .cols button { width: auto }</style>\
         <div class=g>\
           <div style='display: contents'><button id=c style='order: 3'>c</button>\
             <button id=a style='order: 1'>a</button>\
             <div style='display: contents'><button id=d style='order: 4'>d</button>\
               <button id=b style='order: 2'>b</button></div></div></div>\
         <div class=g>\
           <button id=p style='order: 2; position: absolute'>p</button>\
           <button id=q style='order: 5'>q</button><button id=r style='order: 1'>r</button>\
           <button id=s style='order: 1; position: absolute; reading-order: -1'>s</button></div>\
         <div class='g cols' style='reading-flow: grid-columns; \
           grid-template-columns: 50px 50px 50px; \
           grid-template-areas: \"w x x\" \"y y z\"'>\
           <button id=z style='grid-area: z'>z</button><button id=x style='grid-area: x'>x</button>\
           <button id=y style='grid-area: y'>y</button><button id=w style='grid-area: w'>w</button>\
         </div>\
         <span id=host><button id=o5 style='order: 5'>5</button>\
           <button id=o1 style='order: 1'>1</button><button id=o3 style='order: 3'>3</button></span>\
         <script>\
         document.getElementById('host').attachShadow({mode: 'open'}).innerHTML = \
           '<div style=\"display: grid; reading-flow: grid-order\">' + \
           '<button id=o4 style=\"order: 4\">4</button><slot></slot>' + \
           '<button id=o2 style=\"order: 2\">2</button></div>';\
         </script>",
    );
    frame_at(&mut tab, &clock, 16);
    assert_eq!(
        tab_order(&mut tab),
        [
            // Grouped where the first of them is, each group in order.
            "a", "b", "d",
            "c", // In flow first, then the rest by reading-order and tree order.
            "r", "q", "s", "p", // By column, then row, each item's first track.
            "w", "y", "x", "z", // A slot's nodes mixed with the shadow tree's.
            "o1", "o2", "o3", "o4", "o5",
        ]
    );
}

#[test]
fn tab_from_a_focused_scope_owner_goes_into_its_scope() {
    let (mut tab, clock, console) = page(
        "<style>.s { overflow: auto; width: 100px; height: 50px; display: grid; \
         reading-flow: grid-order } .s button { height: 40px }</style>\
         <div id=s class=s><button id=a style='order: 2'>a</button>\
         <button id=b style='order: 1'>b</button></div><button id=after>z</button>\
         <script>document.getElementById('s').focus(); \
         console.log(document.activeElement.id);</script>",
    );
    assert_eq!(console, ["s"]);
    frame_at(&mut tab, &clock, 16);
    assert!(tab.engine.focus_next(false));
    let document = tab.engine.document();
    let id = tab.engine.focused().and_then(|n| document.attr(n, "id"));
    assert_eq!(id, Some("b"));
}

#[test]
fn a_value_range_follows_the_edits_of_its_controls_value() {
    let (_tab, _clock, console) = page(
        "<input id=i value=ABCDEFGHIJ><textarea id=t>0123456789</textarea>\
         <script>\
         const i = document.getElementById('i'), t = document.getElementById('t');\
         const at = (r) => r.startOffset + ',' + r.endOffset + (r.collapsed ? 'c' : '');\
         const r = i.createValueRange(2, 5);\
         console.log(r instanceof OpaqueRange, r instanceof AbstractRange, r instanceof Range,\
           r.startContainer, r.endContainer, at(r));\
         i.setRangeText('Q', 1, 1); console.log(at(r), i.value);\
         i.setRangeText('', 4, 5); console.log(at(r));\
         i.setRangeText('WXYZ', 7, 9); console.log(at(r));\
         i.setRangeText('Z', 1, 6); console.log(at(r), i.value);\
         i.value = i.value; console.log(at(r));\
         const s = t.createValueRange(2, 8);\
         t.value = 'shorter'; console.log(at(s));\
         console.log(at(t.createValueRange(5, 2)));\
         try { t.createValueRange(0, 99); } catch (e) { console.log(e.name); }\
         const c = document.createElement('input'); c.type = 'color';\
         try { c.createValueRange(0, 0); } catch (e) { console.log(e.name); }\
         const u = t.createValueRange(1, 3); u.disconnect(); console.log(at(u));\
         const v = t.createValueRange(1, 3); t.remove(); console.log(at(v));\
         const w = i.createValueRange(1, 3); i.type = 'search'; console.log(at(w));\
         </script>",
    );
    assert_eq!(
        console,
        [
            "true true false undefined undefined 2,5",
            "3,6 AQBCDEFGHIJ",
            "3,5",
            "3,5",
            "1,1c AZGWXYZJ",
            "1,1c",
            "0,0c",
            "5,5c",
            "IndexSizeError",
            "NotSupportedError",
            "0,0c",
            "0,0c",
            "0,0c",
        ]
    );
}

#[test]
fn a_value_range_has_the_boxes_of_its_text_and_is_highlighted() {
    let (mut tab, clock, console) = page(
        "<style>body { margin: 0 } \
         textarea { font: 20px monospace; padding: 0; border: 0; margin: 0 } \
         ::highlight(mark) { background-color: rgb(255, 255, 0) }</style>\
         <textarea id=t cols=20 rows=3>abcdef\nghijkl</textarea>\
         <script>\
         const t = document.getElementById('t');\
         const r = t.createValueRange(2, 9);\
         const rects = r.getClientRects();\
         const b = r.getBoundingClientRect();\
         const caret = t.createValueRange(3, 3);\
         console.log(rects.length, b.width > 0, b.height > 20, caret.getClientRects().length,\
           caret.getBoundingClientRect().width, caret.getBoundingClientRect().height > 0);\
         const one = t.createValueRange(0, 1).getBoundingClientRect();\
         console.log(Math.round(one.width), Math.round(one.left));\
         CSS.highlights.set('mark', new Highlight(t.createValueRange(1, 3)));\
         const hits = CSS.highlights.highlightsFromPoint(one.width * 1.5, one.top + one.height / 2);\
         console.log(hits.length, CSS.highlights.highlightsFromPoint(one.width * 5.5, one.top + 5).length);\
         </script>",
    );
    assert_eq!(console[0], "2 true true 0 0 true");
    let mut cell = console[1]
        .split(' ')
        .map(|n| n.parse::<usize>().expect("a number"));
    let (width, left) = (cell.next().expect("width"), cell.next().expect("left"));
    assert!(width > 0 && left == 0, "{}", console[1]);
    assert_eq!(console[2], "1 0");
    // The second and third characters' cells are yellow, the first's not.
    let frame = frame_at(&mut tab, &clock, 16);
    let yellow = |x0: usize| {
        (0..width)
            .flat_map(|dx| (2..18).map(move |y| (x0 + dx, y)))
            .filter(|&(x, y)| pixel(&frame, x, y) == [255, 255, 0, 255])
            .count()
    };
    assert!(yellow(width) > width * 4, "{}", yellow(width));
    assert!(yellow(2 * width) > width * 4);
    assert_eq!(yellow(0), 0);
}
