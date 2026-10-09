//! The pointer (M26.5, `docs/design/events.md`): its events in the
//! reference browser's order, clicks and their counts at the element
//! over both press and release, activation (links followed by the
//! engine, with scripts on or off), a canceled `pointerdown`, `:active`,
//! the wheel, and a `javascript:` URL's result.

use dom::ElementState;

use super::script_tests::scripted;
use super::tests::load_all;
use crate::testing::*;
use crate::{Button, LoadState, NO_NEW_TAB};

/// The console's lines since they were last taken.
fn lines(tab: &mut Tab) -> Vec<String> {
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

/// Logs every pointer and mouse event at the window, capturing, as
/// `type@target`.
const LOGGER: &str = "<script>\
    ['pointerover','pointerenter','pointerout','pointerleave','pointermove','pointerdown',\
     'pointerup','mouseover','mouseenter','mouseout','mouseleave','mousemove','mousedown',\
     'mouseup','click','auxclick','dblclick','contextmenu'].forEach(function (t) {\
      addEventListener(t, function (e) { \
        console.log(t + '@' + (e.target.id || e.target.nodeName.toLowerCase())); }, true); });\
    </script>";

const PAGE: &str = "<style>body { margin: 0 } \
    #a { position: absolute; left: 0; top: 0; width: 200px; height: 100px } \
    #a1 { margin: 20px; width: 100px; height: 50px } \
    #b { position: absolute; left: 0; top: 120px; width: 200px; height: 100px }</style>\
    <div id=a><div id=a1></div></div><div id=b></div>";

fn page(tab_scripts: bool) -> Tab {
    let (mut tab, _) = scripted(
        Box::leak(format!("{PAGE}{LOGGER}").into_boxed_str()),
        tab_scripts,
    );
    tab.engine.frame(400, 300);
    tab
}

#[test]
fn moving_fires_the_boundary_events_then_the_moves_in_the_reference_browsers_order() {
    let mut tab = page(true);
    tab.engine.pointer_move(150.0, 90.0);
    assert_eq!(
        lines(&mut tab),
        [
            "pointerover@a",
            "pointerenter@html",
            "pointerenter@body",
            "pointerenter@a",
            "mouseover@a",
            "mouseenter@html",
            "mouseenter@body",
            "mouseenter@a",
            "pointermove@a",
            "mousemove@a",
        ]
    );
    // Into a child: the parent is not left.
    tab.engine.pointer_move(40.0, 40.0);
    assert_eq!(
        lines(&mut tab),
        [
            "pointerout@a",
            "pointerover@a1",
            "pointerenter@a1",
            "mouseout@a",
            "mouseover@a1",
            "mouseenter@a1",
            "pointermove@a1",
            "mousemove@a1",
        ]
    );
    tab.engine.pointer_move(45.0, 42.0);
    assert_eq!(lines(&mut tab), ["pointermove@a1", "mousemove@a1"]);
    // Across: the leaves innermost first, the enters outermost first.
    tab.engine.pointer_move(150.0, 150.0);
    assert_eq!(
        lines(&mut tab),
        [
            "pointerout@a1",
            "pointerleave@a1",
            "pointerleave@a",
            "pointerover@b",
            "pointerenter@b",
            "mouseout@a1",
            "mouseleave@a1",
            "mouseleave@a",
            "mouseover@b",
            "mouseenter@b",
            "pointermove@b",
            "mousemove@b",
        ]
    );
    tab.engine.pointer_leave();
    assert_eq!(
        lines(&mut tab),
        [
            "pointerout@b",
            "pointerleave@b",
            "pointerleave@body",
            "pointerleave@html",
            "mouseout@b",
            "mouseleave@b",
            "mouseleave@body",
            "mouseleave@html",
        ]
    );
}

#[test]
fn presses_and_releases_make_clicks_at_the_element_over_both() {
    let mut tab = page(true);
    tab.engine.pointer_move(150.0, 150.0);
    lines(&mut tab);
    tab.engine.pointer_down(150.0, 150.0, Button::Primary, 1);
    tab.engine.pointer_up(150.0, 150.0, Button::Primary, 1);
    assert_eq!(
        lines(&mut tab),
        [
            "pointerdown@b",
            "mousedown@b",
            "pointerup@b",
            "mouseup@b",
            "click@b"
        ]
    );
    tab.engine.pointer_down(150.0, 150.0, Button::Primary, 2);
    tab.engine.pointer_up(150.0, 150.0, Button::Primary, 2);
    assert_eq!(
        lines(&mut tab),
        [
            "pointerdown@b",
            "mousedown@b",
            "pointerup@b",
            "mouseup@b",
            "click@b",
            "dblclick@b"
        ]
    );
    // Pressed on one, released on another: the click is the body's.
    tab.engine.pointer_down(40.0, 40.0, Button::Primary, 1);
    tab.engine.pointer_up(150.0, 150.0, Button::Primary, 1);
    let seen = lines(&mut tab);
    assert_eq!(seen.last().map(String::as_str), Some("click@body"));
    tab.engine.pointer_down(150.0, 150.0, Button::Secondary, 1);
    tab.engine.pointer_up(150.0, 150.0, Button::Secondary, 1);
    assert_eq!(
        lines(&mut tab),
        [
            "pointerdown@b",
            "mousedown@b",
            "pointerup@b",
            "mouseup@b",
            "auxclick@b",
            "contextmenu@b"
        ]
    );
    tab.engine.pointer_down(150.0, 150.0, Button::Auxiliary, 1);
    tab.engine.pointer_up(150.0, 150.0, Button::Auxiliary, 1);
    assert_eq!(
        lines(&mut tab),
        [
            "pointerdown@b",
            "mousedown@b",
            "pointerup@b",
            "mouseup@b",
            "auxclick@b"
        ]
    );
}

#[test]
fn the_events_carry_the_point_the_buttons_and_the_keys() {
    let (mut tab, _) = scripted(
        "<style>body { margin: 0; height: 2000px } \
         #b { position: absolute; left: 10px; top: 130px; width: 100px; height: 100px; \
           border: 5px solid; padding: 3px }</style><div id=b></div><script>\
         var b = document.getElementById('b');\
         ['pointerenter', 'mouseover', 'mousemove', 'pointerdown', 'mousedown', 'click'].forEach(function (t) { \
           b.addEventListener(t, function (e) { console.log([t, e.constructor.name, e.isTrusted, \
             e.clientX, e.clientY, e.pageY, e.offsetX, e.offsetY, e.button, e.buttons, \
             e.detail, e.shiftKey, e.ctrlKey, e.pointerId, e.pointerType, e.pressure, \
             e.view === window, e.bubbles, e.cancelable].join()); }); });\
         </script>",
        true,
    );
    tab.engine.frame(400, 300);
    // Scrolled down by 100: the box's top is 30 in the viewport.
    tab.engine.set_scroll(100.0);
    tab.engine.frame(400, 300);
    tab.engine.set_modifiers(crate::Modifiers {
        shift: true,
        ..Default::default()
    });
    tab.engine.pointer_move(30.0, 50.0);
    tab.engine.pointer_down(30.0, 50.0, Button::Primary, 1);
    tab.engine.pointer_up(30.0, 50.0, Button::Primary, 1);
    let seen = lines(&mut tab);
    assert_eq!(
        seen,
        [
            "pointerenter,PointerEvent,true,30,50,150,15,15,-1,0,0,true,false,1,mouse,0,true,false,false",
            "mouseover,MouseEvent,true,30,50,150,15,15,0,0,0,true,false,,,,true,true,true",
            "mousemove,MouseEvent,true,30,50,150,15,15,0,0,0,true,false,,,,true,true,true",
            "pointerdown,PointerEvent,true,30,50,150,15,15,0,1,0,true,false,1,mouse,0.5,true,true,true",
            "mousedown,MouseEvent,true,30,50,150,15,15,0,1,1,true,false,,,,true,true,true",
            "click,PointerEvent,true,30,50,150,15,15,0,0,1,true,false,1,mouse,0,true,true,true",
        ]
    );
}

#[test]
fn a_canceled_pointerdown_holds_back_the_mouse_events_but_not_the_click() {
    let (mut tab, _) = scripted(
        Box::leak(
            format!(
                "{PAGE}{LOGGER}<script>document.getElementById('b').addEventListener(\
                 'pointerdown', function (e) {{ e.preventDefault(); }});</script>"
            )
            .into_boxed_str(),
        ),
        true,
    );
    tab.engine.frame(400, 300);
    tab.engine.pointer_move(150.0, 150.0);
    lines(&mut tab);
    tab.engine.pointer_down(150.0, 150.0, Button::Primary, 1);
    tab.engine.pointer_move(151.0, 150.0);
    tab.engine.pointer_up(151.0, 150.0, Button::Primary, 1);
    assert_eq!(
        lines(&mut tab),
        ["pointerdown@b", "pointermove@b", "pointerup@b", "click@b"]
    );
    // Once the buttons are up, the mouse events come again.
    tab.engine.pointer_move(152.0, 150.0);
    assert_eq!(lines(&mut tab), ["pointermove@b", "mousemove@b"]);
}

#[test]
fn the_primary_button_held_makes_the_element_and_its_ancestors_active() {
    for scripts in [true, false] {
        let mut tab = page(scripts);
        let document = |tab: &Tab| tab.engine.document().clone();
        let active = |tab: &Tab| -> Vec<String> {
            let document = document(tab);
            document
                .subtree(document.root())
                .filter(|&node| document.state(node).contains(ElementState::ACTIVE))
                .map(|node| document.attr(node, "id").unwrap_or("-").to_owned())
                .collect()
        };
        tab.engine.pointer_down(40.0, 40.0, Button::Primary, 1);
        assert_eq!(active(&tab), ["-", "-", "a", "a1"], "scripts {scripts}");
        tab.engine.pointer_up(150.0, 150.0, Button::Primary, 1);
        assert!(active(&tab).is_empty());
        // Another button activates nothing.
        tab.engine.pointer_down(40.0, 40.0, Button::Secondary, 1);
        assert!(active(&tab).is_empty());
    }
}

#[test]
fn a_click_follows_the_link_it_is_in_unless_canceled() {
    let server = serve(|path| match path {
        "/one.html" => Some(html(
            "<style>body { margin: 0 } a { display: block; height: 40px }</style>\
             <a href=/two.html><span>two</span></a>\
             <a id=c href=/three.html>three</a>\
             <script>document.getElementById('c').addEventListener('click', \
               function (e) { e.preventDefault(); console.log('stopped'); });</script>",
        )),
        "/plain.html" => Some(html(
            "<style>body { margin: 0 } a { display: block; height: 40px }</style>\
             <a href=/two.html>two</a>",
        )),
        "/two.html" => Some(html("<p>two</p>")),
        "/three.html" => Some(html("<p>three</p>")),
        _ => None,
    });
    let two = server.url("/two.html");
    let click = |tab: &mut Tab, y: f32, button: Button| {
        tab.engine.frame(400, 300);
        tab.engine.pointer_down(5.0, y, button, 1);
        tab.engine.pointer_up(5.0, y, button, 1);
    };
    // With scripts on and a listener, off, and on with no script at all
    // in the page: the link is followed all the same.
    for (page, scripts) in [
        ("/one.html", true),
        ("/one.html", false),
        ("/plain.html", true),
    ] {
        let mut tab = Tab::new();
        tab.engine.set_scripting(scripts);
        load_all(&mut tab, server.url(page));
        click(&mut tab, 10.0, Button::Primary);
        tab.wait_until(|engine| {
            engine.url() == Some(&two) && engine.load_state() == &LoadState::Loaded
        });
        assert!(tab.engine.can_go_back(), "{page} {scripts}");
    }
    // Canceled, it goes nowhere; a middle click says why it opens nothing.
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/one.html"));
    click(&mut tab, 50.0, Button::Primary);
    assert_eq!(lines(&mut tab), ["stopped"]);
    click(&mut tab, 10.0, Button::Auxiliary);
    assert_eq!(tab.engine.take_notice().as_deref(), Some(NO_NEW_TAB));
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    assert_eq!(tab.engine.url(), Some(&server.url("/one.html")));
}

#[test]
fn a_wheel_event_canceled_scrolls_nothing() {
    let (mut tab, _) = scripted(
        "<style>body { margin: 0; height: 3000px }</style><div id=d>x</div><script>\
         var stop = false;\
         document.getElementById('d').addEventListener('wheel', function (e) { \
           console.log(e.constructor.name, e.deltaY, e.deltaMode, e.cancelable); \
           if (stop) e.preventDefault(); });\
         </script>",
        true,
    );
    tab.engine.frame(400, 300);
    tab.engine.wheel(5.0, 5.0, 40.0);
    assert_eq!(lines(&mut tab), ["WheelEvent 40 0 true"]);
    assert!(tab.engine.shown.scroll_target > 0.0);
    let before = tab.engine.shown.scroll_target;
    tab.engine.run_classic("stop = true");
    tab.engine.wheel(5.0, 5.0, 40.0);
    assert_eq!(tab.engine.shown.scroll_target, before);
}

#[test]
fn a_javascript_urls_string_is_the_new_document_at_the_same_url() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<style>body { margin: 0 } a { display: block; height: 40px }</style>\
             <a href=\"javascript:'<title>made</title><script>console.log(location.pathname)</script>'\">x</a>",
        )),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    let url = server.url("/page.html");
    load_all(&mut tab, url.clone());
    tab.engine.frame(400, 300);
    tab.engine.pointer_down(5.0, 10.0, Button::Primary, 1);
    tab.engine.pointer_up(5.0, 10.0, Button::Primary, 1);
    tab.wait_until(|engine| engine.title() == "made" && engine.load_state() == &LoadState::Loaded);
    assert_eq!(tab.engine.url(), Some(&url));
    assert!(!tab.engine.can_go_back());
    assert_eq!(lines(&mut tab), ["/page.html"]);
}
