//! Form controls (M26.7, `docs/design/events.md`): their state apart
//! from their attributes, read and set through HTML's IDL, the radio
//! groups and the selects' selectedness, the selection API, a form's
//! controls and a label's, and what each control shows.

use super::script_tests::scripted;
use crate::testing::*;

/// The console's lines since they were last taken.
fn lines(tab: &mut Tab) -> Vec<String> {
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

/// `source` run in the page, and what it logged.
fn run(tab: &mut Tab, source: &str) -> Vec<String> {
    tab.engine.run_classic(source);
    lines(tab)
}

fn page(body: &'static str) -> Tab {
    let (mut tab, _) = scripted(body, true);
    tab.engine.frame(600, 300);
    lines(&mut tab);
    tab
}

/// The texts the frame lays out, in order: from the engine's own
/// layout, kept from frame to frame, not one made afresh.
fn texts(tab: &mut Tab) -> Vec<String> {
    tab.engine.frame(600, 300);
    tab.engine
        .layout(600.0)
        .iter()
        .filter_map(|f| match &f.kind {
            layout::FragmentKind::Text(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn an_inputs_value_is_its_own_once_set_and_sanitized_by_its_type() {
    let mut tab = page(
        "<input id=t value=one><input id=n type=number value=12><input id=c type=checkbox>\
         <input id=h type=hidden value=x><input id=f type=file><input id=col type=color>\
         <input id=e type=email><input id=r type=range min=0 max=10>\
         <script>var $ = function (id) { return document.getElementById(id); };</script>",
    );
    assert_eq!(
        run(
            &mut tab,
            "var t = $('t'); console.log(t.value, t.defaultValue); \
             t.setAttribute('value', 'two'); console.log(t.value); \
             t.value = 'three'; t.setAttribute('value', 'four'); \
             console.log(t.value, t.defaultValue, t.getAttribute('value')); \
             t.value = 'a\\nb\\rc'; console.log(t.value);"
        ),
        ["one one", "two", "three four four", "abc"]
    );
    assert_eq!(
        run(
            &mut tab,
            "var n = $('n'); n.value = 'abc'; console.log(JSON.stringify(n.value)); \
             n.value = '-1.5e3'; console.log(n.value); \
             var col = $('col'); console.log(col.value); col.value = '#ABCDEF'; console.log(col.value); \
             col.value = 'red'; console.log(col.value); \
             var e = $('e'); e.value = '  a@b.c  '; console.log(JSON.stringify(e.value)); \
             var r = $('r'); console.log(r.value); r.value = '20'; console.log(r.value);"
        ),
        [
            "\"\"",
            "-1.5e3",
            "#000000",
            "#abcdef",
            "#000000",
            "\"a@b.c\"",
            "5",
            "10"
        ]
    );
    // The modes but value: the attribute, or `on`; a file input takes
    // only the empty string.
    assert_eq!(
        run(
            &mut tab,
            "var c = $('c'); console.log(c.value); c.value = 'yes'; \
             console.log(c.value, c.getAttribute('value')); \
             var h = $('h'); h.value = 'y'; console.log(h.getAttribute('value')); \
             var f = $('f'); f.value = ''; try { f.value = 'x'; } catch (e) { console.log(e.name); } \
             console.log(t.type, $('n').type, $('c').type); \
             var odd = document.createElement('input'); odd.type = 'FANCY'; \
             console.log(odd.type, odd.getAttribute('type'));"
        ),
        [
            "on",
            "yes yes",
            "y",
            "InvalidStateError",
            "text number checkbox",
            "text FANCY"
        ]
    );
}

#[test]
fn a_textareas_value_is_its_text_until_set() {
    let mut tab = page("<textarea id=a>one\r\ntwo</textarea>");
    assert_eq!(
        run(
            &mut tab,
            "var a = document.getElementById('a'); \
             console.log(JSON.stringify(a.value), a.textLength, a.type); \
             a.defaultValue = 'new'; console.log(a.value); \
             a.value = 'mine'; a.defaultValue = 'later'; \
             console.log(a.value, a.textContent);"
        ),
        ["\"one\\ntwo\" 7 textarea", "new", "mine later"]
    );
}

#[test]
fn checkboxes_and_radio_groups_keep_their_checkedness() {
    let mut tab = page(
        "<style>:checked { margin-left: 1px } :indeterminate { margin-left: 2px }</style>\
         <form><input id=c type=checkbox checked>\
         <input id=a type=radio name=r checked><input id=b type=radio name=r>\
         <input id=x type=radio name=s></form>",
    );
    assert_eq!(
        run(
            &mut tab,
            "var $ = function (id) { return document.getElementById(id); }; \
             var c = $('c'); console.log(c.checked, c.defaultChecked); \
             c.checked = false; c.removeAttribute('checked'); c.setAttribute('checked', ''); \
             console.log(c.checked, c.defaultChecked); \
             $('b').checked = true; console.log($('a').checked, $('b').checked); \
             $('a').defaultChecked = true; console.log($('a').checked, $('b').checked); \
             c.indeterminate = true; console.log(c.indeterminate, c.matches(':indeterminate')); \
             console.log($('x').matches(':indeterminate'), $('a').matches(':indeterminate')); \
             console.log(document.querySelectorAll(':checked').length, \
               c.matches(':checked'), $('b').matches(':checked'));"
        ),
        [
            "true true",
            "false true",
            "false true",
            "false true",
            "true true",
            "true false",
            "1 false true"
        ]
    );
}

#[test]
fn a_select_chooses_its_options_through_its_idl() {
    let mut tab = page(
        "<select id=s><option>a</option><option value=bv selected>b</option>\
         <option id=c>  c\n  see </option></select>\
         <select id=m multiple><option selected>1</option><option>2</option>\
         <option selected>3</option></select>",
    );
    assert_eq!(
        run(
            &mut tab,
            "var s = document.getElementById('s'); var o = s.options; \
             console.log(s.type, s.length, o.length, s.selectedIndex, s.value); \
             console.log(o === s.options, o[2].text, o[2].value, o[2].index, o[1].label); \
             s.selectedIndex = 0; console.log(s.value, o[0].selected, o[1].selected); \
             s.value = 'bv'; console.log(s.selectedIndex); \
             s.value = 'none'; console.log(s.selectedIndex, JSON.stringify(s.value)); \
             o[2].selected = true; console.log(s.selectedIndex, s.selectedOptions.length); \
             var d = document.createElement('option'); d.text = 'd'; s.add(d, 0); \
             console.log(o.length, o[0].text, s.selectedIndex); \
             s.remove(0); o.length = 5; console.log(s.length, o[4].text === ''); \
             o.length = 1; console.log(s.length, o.namedItem('c')); \
             o[0].setAttribute('label', ''); console.log(o[0].label);"
        ),
        [
            "select-one 3 3 1 bv",
            "true c see c see 2 b",
            "a true false",
            "1",
            "-1 \"\"",
            "2 1",
            "4 d 3",
            "5 true",
            "1 null",
            "a",
        ]
    );
    assert_eq!(
        run(
            &mut tab,
            "var m = document.getElementById('m'); \
             console.log(m.type, m.selectedIndex, m.selectedOptions.length, m.value); \
             m.options[1].selected = true; console.log(m.selectedOptions.length); \
             m.selectedIndex = 2; console.log(m.selectedOptions.length, m.selectedIndex);"
        ),
        ["select-multiple 0 2 1", "3", "1 2"]
    );
}

#[test]
fn a_text_controls_selection_is_counted_in_utf16() {
    let mut tab = page("<input id=t value='h\u{e9}llo \u{1f600}x'><input id=c type=checkbox>");
    assert_eq!(
        run(
            &mut tab,
            "var t = document.getElementById('t'); \
             console.log(t.selectionStart, t.selectionEnd, t.selectionDirection); \
             t.setSelectionRange(1, 8, 'backward'); \
             console.log(t.selectionStart, t.selectionEnd, t.selectionDirection); \
             t.setSelectionRange(9, 2); console.log(t.selectionStart, t.selectionEnd); \
             t.select(); console.log(t.selectionStart, t.selectionEnd, t.value.length); \
             t.value = 'ab'; console.log(t.selectionStart, t.selectionEnd); \
             t.selectionStart = 1; console.log(t.selectionStart, t.selectionEnd); \
             var c = document.getElementById('c'); \
             console.log(c.selectionStart); \
             try { c.setSelectionRange(0, 0); } catch (e) { console.log(e.name); }"
        ),
        [
            "0 0 forward",
            "1 8 backward",
            "2 2",
            "0 9 9",
            "2 2",
            "1 2",
            "null",
            "InvalidStateError",
        ]
    );
}

#[test]
fn a_forms_controls_and_a_labels_are_found() {
    let mut tab = page(
        "<form id=f><input name=q id=q><input type=image name=i>\
         <input type=radio name=r value=one><input type=radio name=r value=two checked>\
         <fieldset><select name=s></select></fieldset><label id=l1>L <input id=inner></label>\
         </form><textarea form=f name=t></textarea><label id=l2 for=q>Q</label>\
         <label id=l3 for=nope>N</label><input name=outside>",
    );
    assert_eq!(
        run(
            &mut tab,
            "var f = document.getElementById('f'); var e = f.elements; \
             console.log(f.length, e.length, e === f.elements, e[0].id); \
             console.log(e.namedItem('t').tagName, e.r.constructor.name, e.r.length, e.r.value); \
             e.r.value = 'one'; console.log(e.r[0].checked, e.r[1].checked); \
             console.log(e.i, e.q.form === f); \
             var q = document.getElementById('q'); \
             console.log(document.getElementById('l2').control === q, q.labels.length); \
             console.log(document.getElementById('l1').control.id, \
               document.getElementById('l3').control, document.getElementById('l1').form === f);"
        ),
        [
            "7 7 true q",
            "TEXTAREA RadioNodeList 2 two",
            "true false",
            "undefined true",
            "true 1",
            "inner null true",
        ]
    );
}

#[test]
fn controls_show_their_values_labels_and_choices() {
    let mut tab = page(
        "<input id=t value=hello><input id=p placeholder=hint><input id=w type=password value=abc>\
         <input type=submit><input type=reset><input type=button value=Go>\
         <select id=s><option>one</option><option>two</option></select>\
         <select size=2><option>r1</option><option label=L2>r2</option></select>\
         <textarea id=a>area</textarea>",
    );
    let shown = texts(&mut tab);
    for text in [
        "hello",
        "hint",
        "\u{2022}\u{2022}\u{2022}",
        "Submit",
        "Reset",
        "Go",
        "one",
        "r1",
        "L2",
        "area",
    ] {
        assert!(shown.iter().any(|t| t == text), "{text}: {shown:?}");
    }
    assert!(!shown.iter().any(|t| t == "two" || t == "abc"), "{shown:?}");
    // A script's changes show at the next frame; the placeholder goes
    // when there is a value.
    tab.engine.run_classic(
        "document.getElementById('t').value = 'changed'; \
         document.getElementById('p').value = 'typed'; \
         document.getElementById('s').selectedIndex = 1; \
         document.getElementById('a').value = 'new area';",
    );
    let shown = texts(&mut tab);
    for text in ["changed", "typed", "two", "new"] {
        assert!(shown.iter().any(|t| t == text), "{text}: {shown:?}");
    }
    assert!(
        !shown
            .iter()
            .any(|t| t == "hint" || t == "hello" || t == "one"),
        "{shown:?}"
    );
}

#[test]
fn a_placeholder_is_greyed_and_matches_placeholder_shown() {
    let mut tab =
        page("<style>:placeholder-shown { margin-left: 5px }</style><input id=p placeholder=hint>");
    tab.engine.frame(600, 300);
    let frags = tab.engine.fragments(600.0);
    let hint = frags
        .iter()
        .find(|f| matches!(&f.kind, layout::FragmentKind::Text(t) if t == "hint"))
        .expect("the placeholder");
    let grey = 117.0 / 255.0;
    assert_eq!(
        (hint.style.color.r, hint.style.color.g, hint.style.color.b),
        (grey, grey, grey)
    );
    assert_eq!(
        run(
            &mut tab,
            "var p = document.getElementById('p'); console.log(p.matches(':placeholder-shown'), \
               getComputedStyle(p).color); p.value = 'x'; \
             console.log(p.matches(':placeholder-shown'), getComputedStyle(p).marginLeft);"
        ),
        ["true rgb(0, 0, 0)", "false 0px"]
    );
}

#[test]
fn a_checkbox_and_a_radio_button_show_their_state() {
    let mut tab = page("<input id=c type=checkbox><input id=r type=radio checked disabled>");
    let marks = |tab: &mut Tab| -> Vec<layout::Mark> {
        tab.engine.frame(600, 300);
        tab.engine
            .fragments(600.0)
            .iter()
            .filter_map(|f| match f.kind {
                layout::FragmentKind::Replaced(layout::Replaced::Mark(mark)) => Some(mark),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        marks(&mut tab),
        [
            layout::Mark::Checkbox {
                checked: false,
                indeterminate: false,
                disabled: false
            },
            layout::Mark::Radio {
                checked: true,
                disabled: true
            },
        ]
    );
    tab.engine.run_classic(
        "var c = document.getElementById('c'); c.checked = true; c.indeterminate = true; \
         document.getElementById('r').checked = false;",
    );
    assert_eq!(
        marks(&mut tab),
        [
            layout::Mark::Checkbox {
                checked: true,
                indeterminate: true,
                disabled: false
            },
            layout::Mark::Radio {
                checked: false,
                disabled: true
            },
        ]
    );
}

#[test]
fn a_clone_keeps_its_controls_state() {
    let mut tab = page("<input id=t value=a><input id=c type=checkbox>");
    assert_eq!(
        run(
            &mut tab,
            "var t = document.getElementById('t'); t.value = 'typed'; \
             var c = document.getElementById('c'); c.checked = true; \
             var t2 = t.cloneNode(); var c2 = c.cloneNode(); \
             console.log(t2.value, c2.checked); t.setAttribute('value', 'b'); \
             t2.setAttribute('value', 'c'); console.log(t2.value);"
        ),
        ["typed true", "typed"]
    );
}

#[test]
fn a_clip_is_no_scroll_container() {
    // `overflow: clip`, a text field's (M26.7): the wheel over it
    // scrolls the page, never the box.
    let mut tab = page(
        "<style>body { margin: 0; height: 2000px } \
         #c { overflow: clip; height: 20px; width: 100px; padding-top: 10px } \
         p { height: 200px; margin: 0 }</style>\
         <div id=c><p>tall</p></div><input id=t value=long>",
    );
    let c = by_id(&tab.engine, "c");
    // Over its padding: the box itself is hit.
    assert_eq!(tab.engine.hit_test(10.0, 5.0), Some(c));
    tab.engine.wheel(10.0, 5.0, 40.0);
    assert_eq!(tab.engine.element_scroll(c), 0.0);
    assert!(tab.engine.scroll_target() > 0.0);
}

#[test]
fn a_fields_text_stays_on_one_line() {
    let mut tab =
        page("<input id=t value='one two three four five six seven eight nine ten eleven'>");
    tab.engine.frame(600, 300);
    let t = by_id(&tab.engine, "t");
    let frags = tab.engine.fragments(600.0);
    let lines: Vec<f32> = frags
        .iter()
        .filter(|f| f.node == t && matches!(f.kind, layout::FragmentKind::Text(_)))
        .map(|f| f.y)
        .collect();
    assert!(lines.len() > 1, "{lines:?}");
    assert!(lines.iter().all(|&y| y == lines[0]), "one line: {lines:?}");
}

#[test]
fn a_value_set_shows_in_the_layout_kept() {
    // The one change a frame sees: the layout kept from the last frame
    // forgets the control's box.
    let mut tab =
        page("<p>before</p><div><span>x</span><input id=t value=hello></div><p>after</p>");
    tab.engine
        .run_classic("document.getElementById('t').value = 'changed';");
    let shown = texts(&mut tab);
    assert!(shown.iter().any(|t| t == "changed"), "{shown:?}");
}

#[test]
fn a_focus_scrolls_no_clip_to_its_element() {
    // Focusing scrolls the scroll containers around the element into
    // view (M26.6); a clip is none of them.
    let mut tab = page(
        "<style>body { margin: 0 } div { height: 20px } p { margin: 0; height: 100px }</style>         <div id=c style='overflow: clip'><p>tall</p><input id=i></div>         <div id=h style='overflow: hidden'><p>tall</p><input id=j></div>",
    );
    let (c, h) = (by_id(&tab.engine, "c"), by_id(&tab.engine, "h"));
    tab.engine
        .run_classic("document.getElementById('i').focus();");
    assert_eq!(tab.engine.element_scroll(c), 0.0);
    tab.engine
        .run_classic("document.getElementById('j').focus();");
    assert!(
        tab.engine.element_scroll(h) > 0.0,
        "a hidden box is scrolled"
    );
}
