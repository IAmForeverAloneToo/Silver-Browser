//! Focus and the keyboard (M26.6, `docs/design/events.md`): the
//! sequential focus order and its events in the reference browser's
//! order, `focus()` and `blur()`, the focus flags, a press's focus, the
//! window's focus, autofocus, the focus fixup, and keys: their events,
//! Enter and Space clicking, and the scrolling keys.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use dom::{ElementState, NodeId};

use super::script_tests::scripted;
use super::tests::load_all;
use crate::testing::*;
use crate::{Button, FrameTime, Key, KeyOutcome, LoadState, Modifiers};

/// The console's lines since they were last taken.
fn lines(tab: &mut Tab) -> Vec<String> {
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

/// The element with ID `id`.
fn id(tab: &Tab, id: &str) -> NodeId {
    by_id(&tab.engine, id)
}

/// The ID of the element focused, or `-`.
fn focused(tab: &Tab) -> String {
    let document = tab.engine.document();
    tab.engine.focused().map_or("-".to_owned(), |node| {
        document.attr(node, "id").unwrap_or("?").to_owned()
    })
}

/// Logs every focus event at the window, capturing, as
/// `type@target<related`, and whether it bubbles to the body.
const FOCUS_LOGGER: &str = "<script>\
    ['focus','blur','focusin','focusout'].forEach(function (t) {\
      addEventListener(t, function (e) { \
        var name = function (n) { return n === null ? 'null' : n === window ? 'window' : \
          (n.id || n.nodeName.toLowerCase()); };\
        console.log(t + '@' + name(e.target) + '<' + name(e.relatedTarget) + \
          (e.isTrusted ? '' : ' untrusted') + (e.constructor === FocusEvent ? '' : ' not')); }, true);\
      document.body.addEventListener(t, function (e) { console.log('  ' + t + ' bubbled'); });\
    });\
    </script>";

/// Probe 6's page: the reference browser's Tab order was d1, d2, l1, i1,
/// d0, b1, ta.
const ORDER: &str = "<a id=n>no href</a><a id=l1 href=#x>link</a>\
    <input id=i1><input id=h type=hidden><div id=d0 tabindex=0>d0</div>\
    <div id=dm tabindex=-1>dm</div><div id=d2 tabindex=2>d2</div>\
    <button id=b1>b1</button><button id=bd disabled>bd</button>\
    <button id=bt disabled tabindex=3>bt</button>\
    <div id=d1 tabindex=1>d1</div><div style='display: none'><button id=gone>gone</button></div>\
    <textarea id=ta></textarea>";

fn page(body: &str) -> Tab {
    let (mut tab, _) = scripted(
        Box::leak(format!("{body}{FOCUS_LOGGER}").into_boxed_str()),
        true,
    );
    tab.engine.frame(400, 300);
    lines(&mut tab);
    tab
}

/// A frame with its rendering steps, a frame's time after the last.
fn render(tab: &mut Tab) {
    static FRAMES: AtomicU64 = AtomicU64::new(1);
    let now = Duration::from_millis(16 * FRAMES.fetch_add(1, Ordering::Relaxed));
    tab.engine.frame_at(
        400,
        300,
        FrameTime {
            now,
            deadline: now + Duration::from_millis(16),
        },
    );
}

fn key(name: &str, code: &str) -> Key {
    Key::new(name, code)
}

fn tab_key() -> Key {
    key("Tab", "Tab")
}

#[test]
fn tab_follows_the_sequential_order_and_then_leaves_the_page() {
    let mut tab = page(ORDER);
    let mut order = Vec::new();
    loop {
        let outcome = tab.engine.key_down(&tab_key());
        if outcome == KeyOutcome::FocusLeft {
            break;
        }
        assert_eq!(outcome, KeyOutcome::Taken);
        order.push(focused(&tab));
        assert!(order.len() < 20, "{order:?}");
    }
    assert_eq!(order, ["d1", "d2", "l1", "i1", "d0", "b1", "ta"]);
    assert_eq!(focused(&tab), "-");
    // Shift+Tab from nothing: the last, then back.
    tab.engine.set_modifiers(Modifiers {
        shift: true,
        ..Modifiers::default()
    });
    assert_eq!(tab.engine.key_down(&tab_key()), KeyOutcome::Taken);
    assert_eq!(focused(&tab), "ta");
    tab.engine.key_down(&tab_key());
    assert_eq!(focused(&tab), "b1");
    // Before the first, the page lets the focus go.
    for _ in 0..5 {
        tab.engine.key_down(&tab_key());
    }
    assert_eq!(focused(&tab), "d1");
    assert_eq!(tab.engine.key_down(&tab_key()), KeyOutcome::FocusLeft);
    assert_eq!(focused(&tab), "-");
    // Ctrl+Tab is not the page's to move by.
    tab.engine.set_modifiers(Modifiers {
        ctrl: true,
        ..Modifiers::default()
    });
    assert_eq!(tab.engine.key_down(&tab_key()), KeyOutcome::Ignored);
    assert_eq!(focused(&tab), "-");
}

#[test]
fn focus_events_come_in_the_reference_browsers_order() {
    let mut tab = page(ORDER);
    tab.engine.focus_next(false);
    assert_eq!(
        lines(&mut tab),
        ["focus@d1<null", "focusin@d1<null", "  focusin bubbled"]
    );
    tab.engine.focus_next(false);
    assert_eq!(
        lines(&mut tab),
        [
            "blur@d1<d2",
            "focusout@d1<d2",
            "  focusout bubbled",
            "focus@d2<d1",
            "focusin@d2<d1",
            "  focusin bubbled",
        ]
    );
    // Before the first: the element is blurred, toward nothing.
    tab.engine.focus_next(true);
    lines(&mut tab);
    assert!(!tab.engine.focus_next(true));
    assert_eq!(
        lines(&mut tab),
        ["blur@d1<null", "focusout@d1<null", "  focusout bubbled"]
    );
}

#[test]
fn scripts_focus_and_blur_and_read_the_focus() {
    let mut tab = page(ORDER);
    let run = |tab: &mut Tab, source: &str| {
        tab.engine.run_classic(source);
        lines(tab)
    };
    assert_eq!(
        run(
            &mut tab,
            "console.log(document.activeElement === document.body, document.hasFocus());"
        ),
        ["true true"]
    );
    assert_eq!(
        run(
            &mut tab,
            "document.getElementById('i1').focus(); \
             console.log(document.activeElement.id);"
        ),
        [
            "focus@i1<null",
            "focusin@i1<null",
            "  focusin bubbled",
            "i1"
        ]
    );
    // Not focusable: nothing happens; nor for the element focused.
    for id in ["n", "h", "bd", "bt", "gone", "i1"] {
        assert_eq!(
            run(
                &mut tab,
                &format!("document.getElementById('{id}').focus();")
            ),
            Vec::<String>::new(),
            "{id}"
        );
    }
    assert_eq!(focused(&tab), "i1");
    // tabindex -1 can be focused by a script, from i1.
    assert_eq!(
        run(&mut tab, "document.getElementById('dm').focus();"),
        [
            "blur@i1<dm",
            "focusout@i1<dm",
            "  focusout bubbled",
            "focus@dm<i1",
            "focusin@dm<i1",
            "  focusin bubbled",
        ]
    );
    // blur() on another element does nothing; on the focused, it goes.
    assert_eq!(
        run(&mut tab, "document.getElementById('i1').blur();"),
        Vec::<String>::new()
    );
    assert_eq!(
        run(
            &mut tab,
            "document.getElementById('dm').blur(); \
             console.log(document.activeElement.nodeName);"
        ),
        [
            "blur@dm<null",
            "focusout@dm<null",
            "  focusout bubbled",
            "BODY"
        ]
    );
    // tabIndex: the attribute's, else 0 or -1 as the element can take
    // the focus as it is.
    assert_eq!(
        run(
            &mut tab,
            "console.log(['n','l1','i1','h','d0','dm','d2','b1','bd','ta'].map(function (id) { \
               return document.getElementById(id).tabIndex; }).join(' ')); \
             var p = document.createElement('p'); p.tabIndex = 7; \
             console.log(p.getAttribute('tabindex'), p.tabIndex, document.body.tabIndex);"
        ),
        ["-1 0 0 -1 0 -1 2 0 -1 0", "7 7 -1"]
    );
}

#[test]
fn the_focus_flags_follow_the_focus_and_show_after_keys() {
    let mut tab = page(
        "<style>body { margin: 0 } a, input { display: block; height: 40px }</style>\
         <div id=wrap><a id=l href=#x>l</a><input id=t></div>",
    );
    let state = |tab: &Tab, name: &str| tab.engine.document().state(id(tab, name));
    let focus = ElementState::FOCUS;
    let visible = ElementState::FOCUS_VISIBLE;
    let within = ElementState::FOCUS_WITHIN;
    tab.engine.key_down(&tab_key());
    assert!(state(&tab, "l").contains(focus.with(visible, true)));
    assert!(state(&tab, "wrap").contains(within));
    assert!(!state(&tab, "wrap").contains(focus));
    assert!(
        tab.engine.document().state(tab.engine.document().root()).0 == 0,
        "the document node has no flags"
    );
    // A click on the link focuses it unshown; on the text field, shown.
    tab.engine.key_down(&tab_key());
    tab.engine.pointer_down(5.0, 10.0, Button::Primary, 1);
    tab.engine.pointer_up(5.0, 10.0, Button::Primary, 1);
    assert_eq!(focused(&tab), "l");
    assert!(state(&tab, "l").contains(focus));
    assert!(!state(&tab, "l").contains(visible));
    assert!(!state(&tab, "t").contains(focus));
    tab.engine.pointer_down(5.0, 50.0, Button::Primary, 1);
    assert!(state(&tab, "t").contains(focus.with(visible, true)));
    // A script's focus after the pointer is unshown, after a key shown.
    tab.engine
        .run_classic("document.getElementById('l').focus();");
    assert!(!state(&tab, "l").contains(visible));
    tab.engine.key_down(&key("Shift", "ShiftLeft"));
    tab.engine
        .run_classic("document.getElementById('t').focus(); document.getElementById('l').focus();");
    assert!(state(&tab, "l").contains(visible));
    // A click elsewhere: nothing has the focus, nothing is within.
    tab.engine.pointer_down(300.0, 200.0, Button::Primary, 1);
    assert_eq!(focused(&tab), "-");
    for name in ["l", "t", "wrap"] {
        assert!(
            !state(&tab, name).contains(focus.with(within, true)),
            "{name}"
        );
    }
    // :focus and :focus-within restyle.
    let mut tab = page(
        "<style>#w:focus-within { color: rgb(0, 128, 0) } a:focus { color: rgb(0, 0, 255) }</style>\
         <div id=w><a id=l href=#x>l</a></div>",
    );
    tab.engine.key_down(&tab_key());
    assert_eq!(color(&mut tab.engine, "w"), (0.0, 128.0 / 255.0, 0.0));
    assert_eq!(color(&mut tab.engine, "l"), (0.0, 0.0, 1.0));
    tab.engine.run_classic("document.activeElement.blur();");
    // The link's own colour, and the body's.
    assert_eq!(color(&mut tab.engine, "l"), (0.0, 0.0, 238.0 / 255.0));
    assert_eq!(color(&mut tab.engine, "w"), (0.0, 0.0, 0.0));
}

#[test]
fn a_press_focuses_the_nearest_focusable_unless_mousedown_is_canceled() {
    let mut tab = page(
        "<style>body { margin: 0 } button, div { display: block; height: 40px; width: 200px }</style>\
         <button id=b><span>in a button</span></button><div id=plain>plain</div>\
         <div id=keep tabindex=0>keep</div>\
         <div id=outer tabindex=0 style='height: 80px'><div id=inner>inner</div></div>\
         <script>document.getElementById('keep').addEventListener('mousedown', \
           function (e) { e.preventDefault(); });</script>",
    );
    tab.engine.pointer_down(5.0, 10.0, Button::Primary, 1);
    tab.engine.pointer_up(5.0, 10.0, Button::Primary, 1);
    assert_eq!(focused(&tab), "b");
    // A middle press does not move the focus; a secondary one does.
    tab.engine.pointer_down(5.0, 50.0, Button::Auxiliary, 1);
    assert_eq!(focused(&tab), "b");
    tab.engine.pointer_down(5.0, 50.0, Button::Secondary, 1);
    assert_eq!(focused(&tab), "-");
    tab.engine.pointer_down(5.0, 10.0, Button::Primary, 1);
    assert_eq!(focused(&tab), "b");
    lines(&mut tab);
    // Pressed again: it has the focus, and hears nothing of it.
    tab.engine.pointer_down(5.0, 10.0, Button::Primary, 1);
    assert!(lines(&mut tab).is_empty());
    // Canceled: the focus stays where it was.
    tab.engine.pointer_down(5.0, 90.0, Button::Primary, 1);
    assert_eq!(focused(&tab), "b");
    assert!(lines(&mut tab).is_empty());
    // A block inside a focusable one: the outer takes it.
    tab.engine.pointer_down(5.0, 130.0, Button::Primary, 1);
    assert_eq!(focused(&tab), "outer");
}

#[test]
fn a_listener_that_moves_the_focus_has_the_last_word() {
    let mut tab = page(
        "<div id=a tabindex=0>a</div><div id=b tabindex=0>b</div><div id=c tabindex=0>c</div>\
         <script>document.getElementById('a').addEventListener('blur', function () { \
           document.getElementById('c').focus(); });</script>",
    );
    tab.engine.key_down(&tab_key());
    lines(&mut tab);
    tab.engine.key_down(&tab_key());
    assert_eq!(focused(&tab), "c");
    let log = lines(&mut tab);
    assert!(log.iter().all(|line| !line.contains("@b")), "{log:?}");
}

#[test]
fn the_focus_restyles_with_scripts_off() {
    let (mut tab, _) = scripted(
        "<style>a:focus { color: rgb(0, 0, 255) }</style><a id=l href=#x>l</a>",
        false,
    );
    tab.engine.frame(400, 300);
    assert_eq!(color(&mut tab.engine, "l"), (0.0, 0.0, 238.0 / 255.0));
    tab.engine.key_down(&tab_key());
    assert_eq!(color(&mut tab.engine, "l"), (0.0, 0.0, 1.0));
}

#[test]
fn the_windows_focus_blurs_and_refocuses_the_element() {
    let mut tab = page("<input id=i>");
    tab.engine
        .run_classic("document.getElementById('i').focus();");
    lines(&mut tab);
    tab.engine.set_window_focus(false);
    tab.engine
        .run_classic("console.log(document.hasFocus(), document.activeElement.id);");
    assert_eq!(
        lines(&mut tab),
        ["blur@i<null", "blur@window<null", "false i"]
    );
    tab.engine.set_window_focus(false);
    assert!(lines(&mut tab).is_empty());
    tab.engine.set_window_focus(true);
    assert_eq!(lines(&mut tab), ["focus@window<null", "focus@i<null"]);
    assert_eq!(focused(&tab), "i");
}

/// Logs every key event at the window, capturing.
const KEY_LOGGER: &str = "<script>\
    ['keydown','keypress','keyup','click'].forEach(function (t) {\
      addEventListener(t, function (e) { \
        console.log(t === 'click' ? 'click@' + (e.target.id || e.target.nodeName.toLowerCase()) + \
          ' ' + e.constructor.name + ' ' + e.detail + ' ' + e.isTrusted : \
          [t, JSON.stringify(e.key), e.code, e.location, e.repeat, e.keyCode, e.charCode, \
           e.which, e.shiftKey, e.constructor.name, e.isTrusted, \
           e.target.id || e.target.nodeName.toLowerCase()].join(' ')); }, true); });\
    </script>";

#[test]
fn keys_fire_their_events_at_the_element_focused_or_the_body() {
    let cancel_tab = "<script>\
        document.getElementById('l').addEventListener('keydown', function (e) { \
          if (e.key === 'Tab') e.preventDefault(); });</script>";
    let (mut tab, _) = scripted(
        Box::leak(
            format!("<input id=i><a id=l href=#x>x</a>{KEY_LOGGER}{cancel_tab}").into_boxed_str(),
        ),
        true,
    );
    tab.engine.frame(400, 300);
    tab.engine.key_down(&key("a", "KeyA").with_text("a"));
    tab.engine.key_up(&key("a", "KeyA"));
    assert_eq!(
        lines(&mut tab),
        [
            "keydown \"a\" KeyA 0 false 65 0 65 false KeyboardEvent true body",
            "keypress \"a\" KeyA 0 false 97 97 97 false KeyboardEvent true body",
            "keyup \"a\" KeyA 0 false 65 0 65 false KeyboardEvent true body",
        ]
    );
    tab.engine
        .run_classic("document.getElementById('i').focus();");
    tab.engine.set_modifiers(Modifiers {
        shift: true,
        ..Modifiers::default()
    });
    let mut one = key("1", "Numpad1").with_text("1");
    one.location = 3;
    one.repeat = true;
    tab.engine.key_down(&one);
    tab.engine.set_modifiers(Modifiers::default());
    assert_eq!(
        lines(&mut tab),
        [
            "keydown \"1\" Numpad1 3 true 97 0 97 true KeyboardEvent true i",
            "keypress \"1\" Numpad1 3 true 49 49 49 true KeyboardEvent true i",
        ]
    );
    // Enter's keypress is a carriage return's; Escape and Tab have none,
    // nor a key with Ctrl held.
    let enter = key("Enter", "Enter").with_text("\r");
    tab.engine.key_down(&enter);
    tab.engine
        .key_down(&key("Escape", "Escape").with_text("\u{1b}"));
    tab.engine.set_modifiers(Modifiers {
        ctrl: true,
        ..Modifiers::default()
    });
    tab.engine.key_down(&key("c", "KeyC").with_text("c"));
    tab.engine.set_modifiers(Modifiers::default());
    assert_eq!(
        lines(&mut tab),
        [
            "keydown \"Enter\" Enter 0 false 13 0 13 false KeyboardEvent true i",
            "keypress \"Enter\" Enter 0 false 13 13 13 false KeyboardEvent true i",
            "keydown \"Escape\" Escape 0 false 27 0 27 false KeyboardEvent true i",
            "keydown \"c\" KeyC 0 false 67 0 67 false KeyboardEvent true i",
        ]
    );
    // A canceled keydown has no keypress and no default action: Tab
    // stays on the link.
    tab.engine
        .run_classic("document.getElementById('l').focus();");
    assert_eq!(tab.engine.key_down(&tab_key()), KeyOutcome::Taken);
    assert_eq!(focused(&tab), "l");
    tab.engine.key_down(&key(" ", "Space").with_text(" "));
    let log = lines(&mut tab);
    assert_eq!(log.len(), 3, "{log:?}");
    assert!(
        log[2].starts_with("keypress \" \" Space 0 false 32 32"),
        "{log:?}"
    );
}

#[test]
fn enter_follows_a_link_and_space_clicks_a_button_on_its_release() {
    let server = serve(|path| match path {
        "/one.html" => Some(html(&format!(
            "<a id=l href=/two.html>two</a><button id=b>b</button>\
             <input id=c type=checkbox>{KEY_LOGGER}"
        ))),
        "/two.html" => Some(html("<p>two</p>")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/one.html"));
    tab.engine.frame(400, 300);
    tab.engine
        .run_classic("document.getElementById('b').focus();");
    let space = key(" ", "Space").with_text(" ");
    assert_eq!(tab.engine.key_down(&space), KeyOutcome::Taken);
    let log = lines(&mut tab);
    assert!(log.iter().all(|line| !line.starts_with("click")), "{log:?}");
    tab.engine.key_up(&space);
    let log = lines(&mut tab);
    assert_eq!(
        log.last().map(String::as_str),
        Some("click@b PointerEvent 0 true")
    );
    // Enter clicks a button at once; Space a checkbox, but Enter not.
    tab.engine.key_down(&key("Enter", "Enter"));
    assert!(lines(&mut tab).contains(&"click@b PointerEvent 0 true".to_owned()));
    tab.engine
        .run_classic("document.getElementById('c').focus();");
    tab.engine.key_down(&key("Enter", "Enter"));
    assert!(
        lines(&mut tab)
            .iter()
            .all(|line| !line.starts_with("click"))
    );
    tab.engine.key_down(&space);
    tab.engine.key_up(&space);
    assert!(lines(&mut tab).contains(&"click@c PointerEvent 0 true".to_owned()));
    // A Space released after the focus moved clicks nothing.
    tab.engine.key_down(&space);
    tab.engine
        .run_classic("document.getElementById('b').focus();");
    tab.engine.key_up(&space);
    assert!(
        lines(&mut tab)
            .iter()
            .all(|line| !line.starts_with("click"))
    );
    // Enter on the link: keydown, the click, keypress, then the link
    // followed.
    tab.engine
        .run_classic("document.getElementById('l').focus();");
    lines(&mut tab);
    tab.engine.key_down(&key("Enter", "Enter").with_text("\r"));
    let log = lines(&mut tab);
    assert!(log[0].starts_with("keydown \"Enter\""), "{log:?}");
    assert_eq!(log[1], "click@l PointerEvent 0 true");
    assert!(log[2].starts_with("keypress \"Enter\""), "{log:?}");
    let two = server.url("/two.html");
    tab.wait_until(|engine| {
        engine.url() == Some(&two) && engine.load_state() == &LoadState::Loaded
    });
    // With scripts off, Enter follows a link all the same.
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/one.html"));
    tab.engine.frame(400, 300);
    tab.engine.key_down(&tab_key());
    assert_eq!(
        tab.engine
            .document()
            .attr(tab.engine.focused().unwrap(), "id"),
        Some("l")
    );
    tab.engine.key_down(&key("Enter", "Enter"));
    tab.wait_until(|engine| {
        engine.url() == Some(&two) && engine.load_state() == &LoadState::Loaded
    });
}

#[test]
fn the_scrolling_keys_scroll_unless_a_text_control_has_the_focus() {
    let (mut tab, _) = scripted(
        "<style>body { margin: 0; height: 3000px }</style><input id=i><textarea id=t></textarea>\
         <script>addEventListener('keydown', function (e) { \
           if (e.key === 'PageUp') e.preventDefault(); });</script>",
        true,
    );
    tab.engine.frame(400, 300);
    let scroll = |tab: &mut Tab, name: &str| {
        tab.engine.key_down(&key(name, name));
        tab.engine.scroll_target()
    };
    assert_eq!(scroll(&mut tab, "ArrowDown"), 40.0);
    assert_eq!(scroll(&mut tab, "PageDown"), 40.0 + 260.0);
    assert_eq!(scroll(&mut tab, "ArrowUp"), 260.0);
    // Canceled: nothing.
    assert_eq!(scroll(&mut tab, "PageUp"), 260.0);
    assert_eq!(scroll(&mut tab, "End"), 2700.0);
    assert_eq!(scroll(&mut tab, "Home"), 0.0);
    assert_eq!(scroll(&mut tab, " "), 260.0);
    tab.engine.set_modifiers(Modifiers {
        shift: true,
        ..Modifiers::default()
    });
    assert_eq!(scroll(&mut tab, " "), 0.0);
    tab.engine.set_modifiers(Modifiers {
        alt: true,
        ..Modifiers::default()
    });
    assert_eq!(scroll(&mut tab, "End"), 0.0);
    tab.engine.set_modifiers(Modifiers::default());
    for name in ["i", "t"] {
        tab.engine
            .run_classic(&format!("document.getElementById('{name}').focus();"));
        assert_eq!(scroll(&mut tab, "End"), 0.0, "{name}");
        assert_eq!(scroll(&mut tab, " "), 0.0, "{name}");
    }
}

#[test]
fn autofocus_focuses_the_first_rendered_candidate_once() {
    let mut tab = page(
        "<style>body { margin: 0 } p { height: 2000px }</style>\
         <div style='display: none'><input id=hidden autofocus></div>\
         <p>tall</p><input id=a autofocus><input id=b autofocus>",
    );
    // A frame without the rendering steps does not; the steps do.
    assert_eq!(focused(&tab), "-");
    render(&mut tab);
    assert_eq!(focused(&tab), "a");
    assert_eq!(
        lines(&mut tab),
        ["focus@a<null", "focusin@a<null", "  focusin bubbled"]
    );
    let a = id(&tab, "a");
    assert!(
        tab.engine
            .document()
            .state(a)
            .contains(ElementState::FOCUS_VISIBLE)
    );
    assert!(tab.engine.scroll_target() > 1000.0, "scrolled to it");
    // Once: blurred, nothing takes it back.
    tab.engine.run_classic("document.activeElement.blur();");
    render(&mut tab);
    tab.engine.run_classic(
        "var c = document.createElement('input'); c.id = 'c'; c.autofocus = true; \
         document.body.appendChild(c);",
    );
    render(&mut tab);
    render(&mut tab);
    assert_eq!(focused(&tab), "-");
    // One a script inserts, before anything was focused.
    let mut tab = page("<p>x</p>");
    tab.engine.run_classic(
        "var c = document.createElement('input'); c.id = 'c'; c.setAttribute('autofocus', ''); \
         document.body.appendChild(c);",
    );
    render(&mut tab);
    render(&mut tab);
    assert_eq!(focused(&tab), "c");
}

#[test]
fn the_focus_goes_with_an_element_removed_or_no_longer_rendered() {
    let mut tab = page("<div id=w><input id=i></div><button id=b>b</button>");
    tab.engine
        .run_classic("document.getElementById('i').focus();");
    lines(&mut tab);
    // Removed: no events, as HTML has it.
    tab.engine.run_classic(
        "document.getElementById('w').remove(); \
         console.log(document.activeElement.nodeName);",
    );
    assert_eq!(lines(&mut tab), ["BODY"]);
    assert_eq!(focused(&tab), "-");
    // Hidden or disabled: blurred at the next rendering.
    for change in [
        "document.getElementById('b').style.display = 'none';",
        "document.getElementById('b').disabled = true;",
    ] {
        tab.engine.run_classic(
            "var b = document.getElementById('b'); b.style.display = ''; b.disabled = false; \
             b.focus();",
        );
        render(&mut tab);
        lines(&mut tab);
        tab.engine.run_classic(change);
        assert_eq!(focused(&tab), "b", "{change}");
        render(&mut tab);
        render(&mut tab);
        assert_eq!(focused(&tab), "-", "{change}");
        assert_eq!(
            lines(&mut tab),
            ["blur@b<null", "focusout@b<null", "  focusout bubbled"],
            "{change}"
        );
    }
}

#[test]
fn a_scripts_focus_scrolls_to_the_element_unless_told_not_to() {
    let mut tab = page(
        "<style>body { margin: 0 } p { height: 2000px; margin: 0 }</style>\
         <input id=top><p>tall</p><input id=a><p>tall</p><input id=b>",
    );
    tab.engine
        .run_classic("document.getElementById('b').focus({ preventScroll: true });");
    assert_eq!(focused(&tab), "b");
    assert_eq!(tab.engine.scroll_target(), 0.0);
    tab.engine
        .run_classic("document.getElementById('a').focus();");
    let at = tab.engine.scroll_target();
    assert!((2000.0..=2100.0).contains(&at), "{at}");
    // In view already: it stays.
    tab.engine
        .run_classic("document.getElementById('a').blur(); document.getElementById('a').focus();");
    assert_eq!(tab.engine.scroll_target(), at);
    tab.engine
        .run_classic("document.getElementById('top').focus();");
    assert!(tab.engine.scroll_target() < 100.0);
}

#[test]
fn autofocus_leaves_a_focus_something_has_already() {
    let mut tab = page(
        "<input id=a autofocus><input id=b>\
         <script>document.getElementById('b').focus();</script>",
    );
    render(&mut tab);
    render(&mut tab);
    assert_eq!(focused(&tab), "b");
}

#[test]
fn focus_within_stays_with_the_ancestors_the_focus_moves_between() {
    // css/selectors/focus-within-010: the target is shown only while its
    // wrapper has `:focus-within`, which it keeps as the focus moves
    // from the wrapper to the target, so the target takes it.
    let mut tab = page(
        "<style>#target { display: none } #wrapper:focus-within > #target { display: block }\
         </style><div id=wrapper tabindex=1><div id=target tabindex=2></div></div>\
         <div id=other tabindex=0></div>\
         <script>var w = document.getElementById('wrapper'), \
           t = document.getElementById('target'), o = document.getElementById('other');\
           w.addEventListener('blur', function () { \
             console.log('blur within', w.matches(':focus-within')); });</script>",
    );
    tab.engine.run_classic("w.focus(); t.focus();");
    assert_eq!(focused(&tab), "target");
    assert!(lines(&mut tab).contains(&"blur within true".to_owned()));
    let wrapper = id(&tab, "wrapper");
    assert!(
        tab.engine
            .document()
            .state(wrapper)
            .contains(ElementState::FOCUS_WITHIN),
        "the wrapper holds the focus"
    );
    // Away from the wrapper: it loses `:focus-within`.
    tab.engine.run_classic("o.focus();");
    assert_eq!(focused(&tab), "other");
    assert!(
        !tab.engine
            .document()
            .state(wrapper)
            .contains(ElementState::FOCUS_WITHIN)
    );
    // A focus that fails after the blur leaves no `:focus-within`
    // behind: the target is taken out as the wrapper loses the focus.
    tab.engine.run_classic(
        "w.focus(); w.addEventListener('blur', function () { t.remove(); }); t.focus();",
    );
    assert_eq!(focused(&tab), "-");
    assert!(
        !tab.engine
            .document()
            .state(wrapper)
            .contains(ElementState::FOCUS_WITHIN)
    );
    // The shared ancestor taken out as the focus leaves it takes its
    // `:focus-within` and its ancestors' with it.
    let mut tab = page(
        "<div id=wrapper tabindex=1><div id=target tabindex=2></div></div>\
         <script>var w = document.getElementById('wrapper'), \
           t = document.getElementById('target');</script>",
    );
    tab.engine.run_classic(
        "w.focus(); w.addEventListener('blur', function () { w.remove(); }); t.focus();",
    );
    assert_eq!(focused(&tab), "-");
    let document = tab.engine.document();
    let root = document.document_element().expect("the root");
    assert!(
        document
            .subtree(root)
            .all(|node| !document.state(node).contains(ElementState::FOCUS_WITHIN)),
        "no element keeps it"
    );
    // The engine's own moves keep it too: a click on the target from
    // the wrapper.
    let mut tab = page(
        "<style>#target { display: none; height: 20px } \
         #wrapper:focus-within > #target { display: block }</style>\
         <div id=wrapper tabindex=1><div id=target tabindex=2></div></div>\
         <script>var w = document.getElementById('wrapper');\
           w.addEventListener('blur', function () { \
             console.log('blur within', w.matches(':focus-within')); });</script>",
    );
    tab.engine.run_classic("w.focus();");
    tab.engine.frame(400, 300);
    lines(&mut tab);
    tab.engine.pointer_down(20.0, 15.0, Button::Primary, 1);
    tab.engine.pointer_up(20.0, 15.0, Button::Primary, 1);
    assert_eq!(focused(&tab), "target");
    assert!(lines(&mut tab).contains(&"blur within true".to_owned()));
}
