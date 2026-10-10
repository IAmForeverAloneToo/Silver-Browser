//! Editing (M26.8, `docs/design/events.md`): typing, deleting and moving
//! in text controls with their `beforeinput` and `input`, `maxlength`,
//! lines, `change` on a commit, the clipboard, the pointer's caret and
//! selection, the caret drawn, a field scrolled; checkboxes, radio
//! buttons and labels activated; a select's list and keys.

use paint::DisplayItem;

use super::script_tests::scripted;
use crate::testing::*;
use crate::{Button, Key, KeyOutcome, Modifiers};

/// The console's lines since they were last taken.
fn lines(tab: &mut Tab) -> Vec<String> {
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

/// Logs the editing events at the window, capturing: `type inputType
/// data value` for the input events, `type@target` for the rest.
const LOGGER: &str = "<script>\
    ['beforeinput','input','change','click','keypress'].forEach(function (t) {\
      addEventListener(t, function (e) { \
        var who = e.target.id || e.target.nodeName.toLowerCase();\
        console.log(e.inputType !== undefined && t !== 'click' ? \
          [t, e.inputType, JSON.stringify(e.data), JSON.stringify(e.target.value), \
           e.cancelable, e.isTrusted].join(' ') : \
          t + '@' + who + (/^(checkbox|radio)$/.test(e.target.type) && t !== 'keypress' ? ' ' + e.target.checked : '')); \
      }, true); });\
    </script>";

fn page(body: &str) -> Tab {
    let (mut tab, _) = scripted(Box::leak(format!("{body}{LOGGER}").into_boxed_str()), true);
    tab.engine.frame(600, 300);
    lines(&mut tab);
    tab
}

fn key(name: &str) -> Key {
    Key::new(name, "")
}

fn typed(c: &str) -> Key {
    Key::new(c, "").with_text(c)
}

fn type_text(tab: &mut Tab, text: &str) {
    for c in text.chars() {
        let c = c.to_string();
        tab.engine.key_down(&typed(&c));
        tab.engine.key_up(&key(&c));
    }
}

fn with(tab: &mut Tab, modifiers: Modifiers, f: impl FnOnce(&mut Tab)) {
    tab.engine.set_modifiers(modifiers);
    f(tab);
    tab.engine.set_modifiers(Modifiers::default());
}

const SHIFT: Modifiers = Modifiers {
    shift: true,
    ctrl: false,
    alt: false,
    meta: false,
};
const CTRL: Modifiers = Modifiers {
    shift: false,
    ctrl: true,
    alt: false,
    meta: false,
};

/// `source` run, and what it logged.
fn run(tab: &mut Tab, source: &str) -> Vec<String> {
    tab.engine.run_classic(source);
    lines(tab)
}

fn value(tab: &mut Tab, id: &str) -> String {
    let node = by_id(&tab.engine, id);
    format!("{:?}", dom::forms::value(tab.engine.document(), node))
}

/// A control's selection, in bytes, with its direction as scripts read
/// it.
fn selection(tab: &mut Tab, id: &str) -> String {
    let node = by_id(&tab.engine, id);
    let s = dom::forms::selection(tab.engine.document(), node);
    let direction = match s.direction {
        dom::forms::Direction::Backward => "backward",
        _ => "forward",
    };
    format!("{} {} {direction}", s.start, s.end)
}

#[test]
fn typing_inserts_with_beforeinput_and_input() {
    let mut tab = page("<input id=t value=x>");
    run(&mut tab, "document.getElementById('t').focus();");
    tab.engine.key_down(&key("End"));
    tab.engine.key_down(&typed("a"));
    assert_eq!(
        lines(&mut tab),
        [
            "keypress@t",
            "beforeinput insertText \"a\" \"x\" true true",
            "input insertText \"a\" \"xa\" false true",
        ]
    );
    type_text(&mut tab, "bc");
    assert_eq!(value(&mut tab, "t"), "\"xabc\"");
    assert_eq!(selection(&mut tab, "t"), "4 4 forward");
    // An input method's text, committed.
    assert!(tab.engine.insert_text("\u{65e5}\u{672c}"));
    assert_eq!(value(&mut tab, "t"), "\"xabc\u{65e5}\u{672c}\"");
    lines(&mut tab);
    // A canceled beforeinput changes nothing and gives no input; a
    // canceled keypress types nothing.
    run(
        &mut tab,
        "var t = document.getElementById('t'); \
         t.addEventListener('beforeinput', function (e) { if (e.data === 'q') e.preventDefault(); }); \
         t.addEventListener('keypress', function (e) { if (e.key === 'z') e.preventDefault(); });",
    );
    tab.engine.key_down(&typed("q"));
    tab.engine.key_down(&typed("z"));
    assert_eq!(value(&mut tab, "t"), "\"xabc\u{65e5}\u{672c}\"");
    let log = lines(&mut tab);
    assert!(!log.iter().any(|l| l.starts_with("input ")), "{log:?}");
}

#[test]
fn the_keys_move_select_and_delete() {
    let mut tab = page("<input id=t value='one two three'><input id=r readonly value=ro>");
    run(&mut tab, "document.getElementById('t').focus();");
    tab.engine.key_down(&key("Home"));
    with(&mut tab, CTRL, |tab| {
        tab.engine.key_down(&key("ArrowRight"));
    });
    assert_eq!(selection(&mut tab, "t"), "3 3 forward");
    with(&mut tab, SHIFT, |tab| {
        tab.engine.key_down(&key("ArrowRight"));
        tab.engine.key_down(&key("ArrowRight"));
    });
    assert_eq!(selection(&mut tab, "t"), "3 5 forward");
    tab.engine.key_down(&key("Backspace"));
    assert_eq!(value(&mut tab, "t"), "\"onewo three\"");
    assert_eq!(
        lines(&mut tab)
            .iter()
            .filter(|l| l.starts_with("input"))
            .cloned()
            .collect::<Vec<_>>(),
        ["input deleteContentBackward null \"onewo three\" false true"]
    );
    tab.engine.key_down(&key("Delete"));
    assert_eq!(value(&mut tab, "t"), "\"oneo three\"");
    with(&mut tab, CTRL, |tab| {
        tab.engine.key_down(&key("Backspace"));
    });
    assert_eq!(value(&mut tab, "t"), "\"o three\"");
    with(&mut tab, CTRL, |tab| {
        tab.engine.key_down(&key("a"));
    });
    assert_eq!(selection(&mut tab, "t"), "0 7 forward");
    tab.engine.key_down(&typed("x"));
    assert_eq!(value(&mut tab, "t"), "\"x\"");
    // Backspace at the start changes nothing, and fires nothing.
    tab.engine.key_down(&key("Home"));
    lines(&mut tab);
    tab.engine.key_down(&key("Backspace"));
    assert!(
        lines(&mut tab)
            .iter()
            .all(|l| !l.starts_with("beforeinput"))
    );
    // A readonly field moves and selects, and is not edited.
    run(&mut tab, "document.getElementById('r').focus();");
    tab.engine.key_down(&key("End"));
    tab.engine.key_down(&key("Backspace"));
    tab.engine.key_down(&typed("y"));
    assert_eq!(value(&mut tab, "r"), "\"ro\"");
    assert_eq!(selection(&mut tab, "r"), "2 2 forward");
}

#[test]
fn tab_selects_a_fields_text() {
    let mut tab = page("<input id=t value=abc><textarea id=a>xyz</textarea>");
    tab.engine.key_down(&key("Tab"));
    assert_eq!(selection(&mut tab, "t"), "0 3 forward");
    tab.engine.key_down(&key("Tab"));
    assert_eq!(
        selection(&mut tab, "a"),
        "0 0 forward",
        "a textarea's is not"
    );
}

#[test]
fn maxlength_cuts_what_is_typed_and_pasted_in_utf16_units() {
    let mut tab = page("<input id=t maxlength=4 value=ab>");
    run(&mut tab, "document.getElementById('t').focus();");
    tab.engine.key_down(&key("End"));
    assert!(tab.engine.paste("\u{1f600}x"));
    assert_eq!(value(&mut tab, "t"), "\"ab\u{1f600}\"");
    tab.engine.key_down(&typed("z"));
    assert_eq!(value(&mut tab, "t"), "\"ab\u{1f600}\"");
    let log = lines(&mut tab);
    assert!(
        log.contains(&"input insertFromPaste \"\u{1f600}\" \"ab\u{1f600}\" false true".to_owned()),
        "{log:?}"
    );
}

#[test]
fn a_textarea_has_lines() {
    let mut tab = page("<textarea id=a>one\ntwo\nthree</textarea>");
    run(
        &mut tab,
        "var a = document.getElementById('a'); a.focus(); a.setSelectionRange(5, 5);",
    );
    tab.engine.key_down(&key("ArrowUp"));
    assert_eq!(selection(&mut tab, "a"), "1 1 forward");
    tab.engine.key_down(&key("ArrowDown"));
    tab.engine.key_down(&key("ArrowDown"));
    assert_eq!(selection(&mut tab, "a"), "9 9 forward");
    tab.engine.key_down(&key("ArrowDown"));
    assert_eq!(
        selection(&mut tab, "a"),
        "13 13 forward",
        "past the last line, its end"
    );
    tab.engine.key_down(&key("Home"));
    assert_eq!(selection(&mut tab, "a"), "8 8 forward");
    tab.engine.key_down(&key("Enter"));
    assert_eq!(value(&mut tab, "a"), "\"one\\ntwo\\n\\nthree\"");
    let log = lines(&mut tab);
    assert!(
        log.iter()
            .any(|l| l.starts_with("input insertLineBreak null")),
        "{log:?}"
    );
    // Lines past its rows scroll it to the caret, and back.
    let a = by_id(&tab.engine, "a");
    tab.engine.frame(600, 300);
    for _ in 0..6 {
        tab.engine.key_down(&key("Enter"));
    }
    tab.engine.frame(600, 300);
    assert!(tab.engine.element_scroll(a) > 0.0);
    with(&mut tab, CTRL, |tab| {
        tab.engine.key_down(&key("Home"));
    });
    assert_eq!(tab.engine.element_scroll(a), 0.0);
}

#[test]
fn change_fires_when_an_edit_is_committed() {
    let mut tab = page("<input id=t value=a><input id=u><textarea id=x></textarea>");
    let changes = |tab: &mut Tab| -> Vec<String> {
        lines(tab)
            .into_iter()
            .filter(|l| l.starts_with("change"))
            .collect()
    };
    run(&mut tab, "document.getElementById('t').focus();");
    type_text(&mut tab, "b");
    assert!(changes(&mut tab).is_empty());
    tab.engine.key_down(&key("Tab"));
    assert_eq!(changes(&mut tab), ["change@t"]);
    // Typed and taken back: no change; a script's value: no change.
    tab.engine.key_down(&typed("c"));
    tab.engine.key_down(&key("Backspace"));
    lines(&mut tab);
    let log = run(&mut tab, "document.getElementById('x').focus();");
    assert!(log.iter().all(|l| !l.starts_with("change")), "{log:?}");
    run(
        &mut tab,
        "document.getElementById('u').focus(); document.getElementById('u').value = 'v';",
    );
    run(&mut tab, "document.getElementById('u').blur();");
    assert!(changes(&mut tab).is_empty());
    // Enter commits a field's edit; a script's blur() too.
    run(&mut tab, "document.getElementById('u').focus();");
    type_text(&mut tab, "w");
    tab.engine
        .key_down(&Key::new("Enter", "Enter").with_text("\r"));
    assert_eq!(changes(&mut tab), ["change@u"]);
    type_text(&mut tab, "w");
    lines(&mut tab);
    let log = run(&mut tab, "document.getElementById('u').blur();");
    assert_eq!(log, ["change@u"]);
}

#[test]
fn the_clipboard_copies_cuts_and_pastes() {
    let mut tab = page("<input id=t value='hello world'><input id=p type=password value=secret>");
    run(
        &mut tab,
        "var t = document.getElementById('t'); t.focus(); t.setSelectionRange(0, 5);",
    );
    assert_eq!(tab.engine.copy().as_deref(), Some("hello"));
    lines(&mut tab);
    assert_eq!(tab.engine.cut().as_deref(), Some("hello"));
    assert_eq!(value(&mut tab, "t"), "\" world\"");
    let log = lines(&mut tab);
    assert!(
        log.contains(&"input deleteByCut null \" world\" false true".to_owned()),
        "{log:?}"
    );
    assert!(tab.engine.paste("HI"));
    assert_eq!(value(&mut tab, "t"), "\"HI world\"");
    assert_eq!(tab.engine.copy(), None, "nothing selected");
    run(
        &mut tab,
        "var p = document.getElementById('p'); p.focus(); p.select();",
    );
    assert_eq!(tab.engine.copy(), None, "a password is not copied");
    run(&mut tab, "document.activeElement.blur();");
    assert!(!tab.engine.paste("x"));
}

/// The field's box, as the frame lays it out.
fn field_box(tab: &mut Tab, id: &str) -> layout::Fragment {
    tab.engine.frame(600, 300);
    let node = by_id(&tab.engine, id);
    tab.engine
        .layout(600.0)
        .iter()
        .find(|f| f.node == node && matches!(f.kind, layout::FragmentKind::Box))
        .cloned()
        .expect("the field's box")
}

#[test]
fn the_pointer_places_the_caret_and_drags_the_selection() {
    let mut tab = page("<style>body { margin: 0 }</style><input id=t value='alpha beta'>");
    let b = field_box(&mut tab, "t");
    let (x0, y) = (b.x + 4.0, b.y + b.h / 2.0);
    // At the right of the text: the caret at its end.
    tab.engine
        .pointer_down(b.x + b.w - 5.0, y, Button::Primary, 1);
    tab.engine
        .pointer_up(b.x + b.w - 5.0, y, Button::Primary, 1);
    assert_eq!(selection(&mut tab, "t"), "10 10 forward");
    // At the left: the start; dragged right, a selection.
    tab.engine.pointer_down(x0, y, Button::Primary, 1);
    tab.engine.pointer_move(b.x + b.w - 5.0, y);
    tab.engine
        .pointer_up(b.x + b.w - 5.0, y, Button::Primary, 1);
    assert_eq!(selection(&mut tab, "t"), "0 10 forward");
    // Shift extends from the caret's other end; a second press selects
    // the word.
    tab.engine.pointer_down(x0, y, Button::Primary, 1);
    tab.engine.pointer_up(x0, y, Button::Primary, 1);
    with(&mut tab, SHIFT, |tab| {
        tab.engine
            .pointer_down(b.x + b.w - 5.0, y, Button::Primary, 1);
        tab.engine
            .pointer_up(b.x + b.w - 5.0, y, Button::Primary, 1);
    });
    assert_eq!(selection(&mut tab, "t"), "0 10 forward");
    tab.engine.pointer_down(x0, y, Button::Primary, 2);
    assert_eq!(selection(&mut tab, "t"), "0 5 forward");
}

/// The caret's and the selection's items over the frame.
fn caret(tab: &mut Tab) -> Vec<(f32, f32, f32)> {
    tab.engine.frame(600, 300);
    tab.engine
        .caret_items()
        .into_iter()
        .filter_map(|item| match item {
            DisplayItem::Rect { x, w, color, .. } => Some((x, w, f32::from(color.a))),
            _ => None,
        })
        .collect()
}

#[test]
fn the_caret_and_the_selection_are_drawn_in_the_field() {
    let mut tab = page("<style>body { margin: 0 }</style><input id=t value=abc>");
    assert!(caret(&mut tab).is_empty(), "nothing focused");
    run(&mut tab, "document.getElementById('t').focus();");
    tab.engine.key_down(&key("End"));
    let b = field_box(&mut tab, "t");
    let bar = caret(&mut tab);
    assert_eq!(bar.len(), 1);
    assert_eq!((bar[0].1, bar[0].2), (1.0, 255.0), "a 1px opaque bar");
    assert!(bar[0].0 > b.x + 20.0, "after the text: {bar:?}");
    run(
        &mut tab,
        "document.getElementById('t').setSelectionRange(0, 2);",
    );
    let band = caret(&mut tab);
    assert_eq!(band.len(), 1);
    assert!(
        band[0].1 > 5.0 && band[0].2 < 255.0,
        "a band let through: {band:?}"
    );
    // The window without the system's focus: nothing.
    tab.engine.set_window_focus(false);
    assert!(caret(&mut tab).is_empty());
}

#[test]
fn a_field_scrolls_to_keep_its_caret_in_view() {
    let mut tab = page("<style>body { margin: 0 }</style><input id=t size=5>");
    run(&mut tab, "document.getElementById('t').focus();");
    let b = field_box(&mut tab, "t");
    type_text(&mut tab, "abcdefghijklmnopqrstuvwxyz");
    let bar = caret(&mut tab);
    assert!(bar[0].0 <= b.x + b.w, "the caret in view: {bar:?} {b:?}");
    let t = by_id(&tab.engine, "t");
    let scroll = tab.engine.document().control(t).map_or(0, |c| c.scroll);
    assert!(scroll > 0);
    // The text moved left with it.
    tab.engine.frame(600, 300);
    let first = tab
        .engine
        .layout(600.0)
        .iter()
        .find(|f| f.node == t && matches!(f.kind, layout::FragmentKind::Text(_)))
        .map(|f| f.x)
        .expect("the text");
    assert!(first < b.x, "{first} {}", b.x);
    tab.engine.key_down(&key("Home"));
    let t = by_id(&tab.engine, "t");
    assert_eq!(tab.engine.document().control(t).map_or(1, |c| c.scroll), 0);
}

#[test]
fn checkboxes_radios_and_labels_are_activated() {
    let mut tab = page(
        "<style>body { margin: 0 } input, label { display: block; margin: 0 }</style>\
         <input id=c type=checkbox><input id=a type=radio name=r checked>\
         <input id=b type=radio name=r><label id=l for=c>label</label>\
         <label id=lt>text <input id=t></label>\
         <script>var stop = false; document.getElementById('c').addEventListener('click', \
           function (e) { console.log('listener sees ' + e.target.checked); \
           if (stop) e.preventDefault(); });</script>",
    );
    assert_eq!(
        run(&mut tab, "document.getElementById('c').click();"),
        [
            "click@c true",
            "listener sees true",
            "input@c true",
            "change@c true"
        ]
    );
    // Canceled: put back, no input nor change.
    assert_eq!(
        run(
            &mut tab,
            "stop = true; document.getElementById('c').click(); stop = false;"
        ),
        ["click@c false", "listener sees false"]
    );
    assert_eq!(
        run(
            &mut tab,
            "console.log(document.getElementById('c').checked)"
        ),
        ["true"]
    );
    run(&mut tab, "document.getElementById('b').click();");
    assert_eq!(
        run(
            &mut tab,
            "console.log(document.getElementById('a').checked, document.getElementById('b').checked)"
        ),
        ["false true"]
    );
    // A label's click is its control's; a text field's is focused too.
    let log = run(&mut tab, "document.getElementById('l').click();");
    assert!(log.contains(&"change@c false".to_owned()), "{log:?}");
    run(&mut tab, "document.getElementById('lt').click();");
    assert_eq!(
        run(&mut tab, "console.log(document.activeElement.id)"),
        ["t"]
    );
    // The pointer's click too.
    let b = field_box(&mut tab, "c");
    tab.engine
        .pointer_down(b.x + 5.0, b.y + 5.0, Button::Primary, 1);
    tab.engine
        .pointer_up(b.x + 5.0, b.y + 5.0, Button::Primary, 1);
    let log = run(
        &mut tab,
        "console.log(document.getElementById('c').checked)",
    );
    assert_eq!(
        log,
        [
            "click@c true",
            "listener sees true",
            "input@c true",
            "change@c true",
            "true"
        ]
    );
}

#[test]
fn with_scripts_off_a_click_toggles_and_a_label_forwards() {
    let (mut tab, _) = scripted(
        "<style>body { margin: 0 } input, label { display: block; height: 20px; margin: 0 }</style>\
         <input id=c type=checkbox><label for=c id=l>label</label>",
        false,
    );
    tab.engine.frame(600, 300);
    let c = by_id(&tab.engine, "c");
    let click = |tab: &mut Tab, y: f32| {
        tab.engine.pointer_down(5.0, y, Button::Primary, 1);
        tab.engine.pointer_up(5.0, y, Button::Primary, 1);
    };
    click(&mut tab, 5.0);
    assert!(dom::forms::checkedness(tab.engine.document(), c));
    click(&mut tab, 25.0);
    assert!(!dom::forms::checkedness(tab.engine.document(), c));
}

#[test]
fn a_drop_downs_list_opens_and_chooses() {
    let mut tab = page(
        "<style>body { margin: 0 }</style><select id=s><option>one</option>\
         <option disabled>two</option><option>three</option></select>",
    );
    let b = field_box(&mut tab, "s");
    let s = by_id(&tab.engine, "s");
    tab.engine
        .pointer_down(b.x + 5.0, b.y + 5.0, Button::Primary, 1);
    tab.engine
        .pointer_up(b.x + 5.0, b.y + 5.0, Button::Primary, 1);
    assert_eq!(tab.engine.popup_open(), Some(s), "a press opens it");
    tab.engine.frame(600, 300);
    let texts: Vec<String> = tab
        .engine
        .popup_items()
        .into_iter()
        .filter_map(|item| match item {
            DisplayItem::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["one", "two", "three"]);
    lines(&mut tab);
    // Down skips the disabled option; Enter chooses.
    assert_eq!(tab.engine.key_down(&key("ArrowDown")), KeyOutcome::Taken);
    tab.engine.key_down(&key("Enter"));
    assert_eq!(tab.engine.popup_open(), None);
    assert_eq!(lines(&mut tab), ["input@s", "change@s"]);
    assert_eq!(value(&mut tab, "s"), "\"three\"");
    // Escape closes with no choice; a press outside too, unheard.
    tab.engine.key_down(&key(" "));
    assert_eq!(tab.engine.popup_open(), Some(s));
    tab.engine.key_down(&key("ArrowUp"));
    tab.engine.key_down(&key("Escape"));
    assert_eq!(tab.engine.popup_open(), None);
    assert_eq!(value(&mut tab, "s"), "\"three\"");
    with(
        &mut tab,
        Modifiers {
            alt: true,
            ..Modifiers::default()
        },
        |tab| {
            tab.engine.key_down(&key("ArrowDown"));
        },
    );
    assert_eq!(tab.engine.popup_open(), Some(s));
    tab.engine.pointer_down(500.0, 250.0, Button::Primary, 1);
    tab.engine.pointer_up(500.0, 250.0, Button::Primary, 1);
    assert_eq!(tab.engine.popup_open(), None);
    assert!(lines(&mut tab).is_empty());
    // A row released on is chosen.
    tab.engine
        .pointer_down(b.x + 5.0, b.y + 5.0, Button::Primary, 1);
    tab.engine
        .pointer_up(b.x + 5.0, b.y + 5.0, Button::Primary, 1);
    let row_y = b.y + b.h + 5.0;
    tab.engine.pointer_move(b.x + 5.0, row_y);
    tab.engine
        .pointer_down(b.x + 5.0, row_y, Button::Primary, 1);
    tab.engine.pointer_up(b.x + 5.0, row_y, Button::Primary, 1);
    assert_eq!(value(&mut tab, "s"), "\"one\"");
    // A disabled row released on chooses nothing.
    let row = (tab
        .engine
        .layout_fonts
        .primary()
        .metrics(13.3333)
        .line_height()
        + 2.0)
        .ceil();
    tab.engine
        .pointer_down(b.x + 5.0, b.y + 5.0, Button::Primary, 1);
    tab.engine
        .pointer_up(b.x + 5.0, b.y + 5.0, Button::Primary, 1);
    tab.engine
        .pointer_down(b.x + 5.0, row_y + row, Button::Primary, 1);
    tab.engine
        .pointer_up(b.x + 5.0, row_y + row, Button::Primary, 1);
    assert_eq!(value(&mut tab, "s"), "\"one\"");
    assert_eq!(tab.engine.popup_open(), None);
    // Closed, the arrows change the choice at once.
    lines(&mut tab);
    tab.engine.key_down(&key("ArrowDown"));
    assert_eq!(value(&mut tab, "s"), "\"three\"");
    assert_eq!(lines(&mut tab), ["input@s", "change@s"]);
    tab.engine.key_down(&key("ArrowDown"));
    assert!(lines(&mut tab).is_empty(), "already the last");
}

#[test]
fn a_list_box_takes_presses_on_its_options() {
    let mut tab = page(
        "<style>body { margin: 0 }</style>\
         <select id=m multiple size=3><option id=o1>one</option><option id=o2>two</option>\
         <option id=o3>three</option></select>",
    );
    let press = |tab: &mut Tab, id: &str| {
        let b = field_box(tab, id);
        tab.engine
            .pointer_down(b.x + 3.0, b.y + 3.0, Button::Primary, 1);
        tab.engine
            .pointer_up(b.x + 3.0, b.y + 3.0, Button::Primary, 1);
    };
    let chosen = |tab: &mut Tab| {
        run(
            tab,
            "console.log(Array.prototype.map.call(document.getElementById('m').selectedOptions, \
               function (o) { return o.id; }).join(' '))",
        )
        .pop()
        .expect("the options")
    };
    press(&mut tab, "o2");
    let log = lines(&mut tab);
    assert!(log.contains(&"change@m".to_owned()), "{log:?}");
    assert_eq!(chosen(&mut tab), "o2");
    with(&mut tab, CTRL, |tab| press(tab, "o3"));
    assert_eq!(chosen(&mut tab), "o2 o3");
    press(&mut tab, "o1");
    assert_eq!(chosen(&mut tab), "o1");
}

#[test]
fn shortcuts_type_nothing_and_a_scripts_value_drops_the_edit() {
    let mut tab = page("<input id=t>");
    run(&mut tab, "document.getElementById('t').focus();");
    with(&mut tab, CTRL, |tab| {
        tab.engine.key_down(&typed("b"));
    });
    assert_eq!(value(&mut tab, "t"), "\"\"");
    // Typed, then the value set by a script: nothing of the user's to
    // commit.
    type_text(&mut tab, "q");
    run(&mut tab, "document.getElementById('t').value = 'set';");
    lines(&mut tab);
    let log = run(&mut tab, "document.getElementById('t').blur();");
    assert!(log.is_empty(), "{log:?}");
}

#[test]
fn the_caret_stands_after_the_text_and_follows_the_scroll() {
    let mut tab =
        page("<style>body { margin: 0 }</style><input id=t value=abc><input id=s size=5>");
    run(&mut tab, "document.getElementById('t').focus();");
    tab.engine.key_down(&key("End"));
    let b = field_box(&mut tab, "t");
    let size = 13.3333;
    // The 2px border and the 2px padding, then the text.
    let want = (b.x + 4.0 + tab.engine.layout_fonts.measure("abc", size)).floor();
    assert_eq!(caret(&mut tab)[0].0, want);
    // A press at the right of a scrolled field: the caret at the end.
    run(&mut tab, "document.getElementById('s').focus();");
    type_text(&mut tab, "abcdefghijklmnopqrstuvwxyz");
    let b = field_box(&mut tab, "s");
    tab.engine.key_down(&key("Home"));
    tab.engine.key_down(&key("End"));
    tab.engine
        .pointer_down(b.x + b.w - 4.0, b.y + b.h / 2.0, Button::Primary, 1);
    tab.engine
        .pointer_up(b.x + b.w - 4.0, b.y + b.h / 2.0, Button::Primary, 1);
    assert_eq!(selection(&mut tab, "s"), "26 26 forward");
}

#[test]
fn a_controls_own_click_in_its_label_is_one_click() {
    let mut tab = page(
        "<style>body { margin: 0 } input { display: block }</style>\
                        <label><input id=c type=checkbox></label>",
    );
    let b = field_box(&mut tab, "c");
    tab.engine
        .pointer_down(b.x + 5.0, b.y + 5.0, Button::Primary, 1);
    tab.engine
        .pointer_up(b.x + 5.0, b.y + 5.0, Button::Primary, 1);
    let log = lines(&mut tab);
    assert_eq!(
        log.iter().filter(|l| l.starts_with("click")).count(),
        1,
        "{log:?}"
    );
    assert_eq!(
        run(
            &mut tab,
            "console.log(document.getElementById('c').checked)"
        ),
        ["true"]
    );
}

#[test]
fn a_radio_is_checked_with_scripts_off_and_a_scroll_closes_a_list() {
    let (mut tab, _) = scripted(
        "<style>body { margin: 0; height: 2000px } input { display: block; height: 20px; margin: 0 }</style>\
         <input id=r type=radio name=g><select id=s><option>a</option><option>b</option></select>",
        false,
    );
    tab.engine.frame(600, 300);
    let r = by_id(&tab.engine, "r");
    for _ in 0..2 {
        tab.engine.pointer_down(5.0, 5.0, Button::Primary, 1);
        tab.engine.pointer_up(5.0, 5.0, Button::Primary, 1);
        assert!(
            dom::forms::checkedness(tab.engine.document(), r),
            "and stays checked"
        );
    }
    let b = field_box(&mut tab, "s");
    tab.engine
        .pointer_down(b.x + 3.0, b.y + 3.0, Button::Primary, 1);
    tab.engine
        .pointer_up(b.x + 3.0, b.y + 3.0, Button::Primary, 1);
    assert!(tab.engine.popup_open().is_some());
    tab.engine.wheel(300.0, 200.0, 40.0);
    assert_eq!(tab.engine.popup_open(), None);
}

#[test]
fn a_readonly_field_takes_no_paste_nor_committed_text() {
    let mut tab = page("<input id=r readonly value=ro>");
    run(&mut tab, "document.getElementById('r').focus();");
    assert!(tab.engine.paste("x"));
    assert!(tab.engine.insert_text("y"));
    assert_eq!(value(&mut tab, "r"), "\"ro\"");
}

#[test]
fn a_press_elsewhere_ends_a_fields_drag() {
    let mut tab = page(
        "<style>body { margin: 0 } input, div { display: block; height: 30px }</style>\
         <input id=t value='alpha beta'><div id=d>d</div>\
         <script>document.getElementById('d').addEventListener('mousedown', \
           function (e) { e.preventDefault(); });</script>",
    );
    let b = field_box(&mut tab, "t");
    let y = b.y + b.h / 2.0;
    tab.engine.pointer_down(b.x + 4.0, y, Button::Primary, 1);
    tab.engine.pointer_up(b.x + 4.0, y, Button::Primary, 1);
    assert_eq!(selection(&mut tab, "t"), "0 0 forward");
    // The press on the div keeps the field's focus; its drag is none of
    // the field's.
    tab.engine.pointer_down(5.0, 45.0, Button::Primary, 1);
    tab.engine.pointer_move(b.x + b.w - 5.0, y);
    tab.engine
        .pointer_up(b.x + b.w - 5.0, y, Button::Primary, 1);
    assert_eq!(selection(&mut tab, "t"), "0 0 forward");
}

#[test]
fn a_click_on_a_field_in_its_label_is_one_click() {
    let mut tab = page(
        "<style>body { margin: 0 } input { display: block }</style>\
                        <label>text <input id=t></label>",
    );
    let b = field_box(&mut tab, "t");
    tab.engine
        .pointer_down(b.x + 5.0, b.y + 5.0, Button::Primary, 1);
    tab.engine
        .pointer_up(b.x + 5.0, b.y + 5.0, Button::Primary, 1);
    let log = lines(&mut tab);
    assert_eq!(
        log.iter().filter(|l| l.starts_with("click")).count(),
        1,
        "{log:?}"
    );
}
