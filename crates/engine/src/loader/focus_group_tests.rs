//! Focusgroups, the sequential focus navigation starting point, popover
//! invokers' scopes, autofocus candidates and what scripts ask of focus
//! (gap batch H): the arrow keys, Home and End in a focusgroup, its
//! segments and memory for Tab, a radio group's arrows, Tab from where
//! an element left the tree, `FocusOptions.focusVisible`, and the
//! contents a `content-visibility` element skips for its observers.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use dom::ElementState;

use super::script_tests::scripted;
use crate::testing::*;
use crate::{FrameTime, Key, Modifiers};

fn page(body: &str) -> Tab {
    let (mut tab, _) = scripted(Box::leak(body.to_owned().into_boxed_str()), true);
    tab.engine.frame(400, 300);
    tab.engine.take_console();
    tab
}

/// The ID of the element focused, or `-`.
fn focused(tab: &Tab) -> String {
    let document = tab.engine.document();
    tab.engine.focused().map_or("-".to_owned(), |node| {
        document.attr(node, "id").unwrap_or("?").to_owned()
    })
}

/// Focuses the element with ID `id` by a script.
fn focus(tab: &mut Tab, id: &str) {
    crate::differential::run_script(
        &mut tab.engine,
        &format!("document.getElementById('{id}').focus()"),
    );
}

/// Presses and releases the key named `name`, Shift held for `shift`.
fn press(tab: &mut Tab, name: &str, shift: bool) {
    tab.engine.set_modifiers(Modifiers {
        shift,
        ..Modifiers::default()
    });
    let key = Key::new(name, name);
    tab.engine.key_down(&key);
    tab.engine.key_up(&key);
    tab.engine.set_modifiers(Modifiers::default());
}

/// Where the key `name` takes the focus from the element `from`.
fn from(tab: &mut Tab, id: &str, name: &str) -> String {
    focus(tab, id);
    press(tab, name, false);
    focused(tab)
}

/// The elements Tab (Shift+Tab, `backward`) reaches from `start`, until
/// the page lets the focus go.
fn tabs(tab: &mut Tab, start: &str, backward: bool) -> Vec<String> {
    focus(tab, start);
    let mut order = Vec::new();
    for _ in 0..12 {
        press(tab, "Tab", backward);
        let now = focused(tab);
        if now == "-" {
            break;
        }
        order.push(now);
    }
    order
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

/// The console's lines since they were last taken.
fn lines(tab: &mut Tab) -> Vec<String> {
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

#[test]
fn arrows_move_along_the_axes_a_focusgroup_takes() {
    let mut tab = page(
        "<div focusgroup=toolbar><button id=a>a</button><span id=x>x</span>\
         <div><button id=b>b</button></div><button id=c>c</button></div>\
         <div focusgroup='menu'><button id=m1>1</button><button id=m2>2</button></div>\
         <div focusgroup='toolbar block inline wrap'><button id=w1>1</button>\
         <button id=w2 tabindex=-1>2</button><button id=w3>3</button></div>",
    );
    // A toolbar's inline axis, past what cannot take the focus and into
    // descendants; no wrapping.
    assert_eq!(from(&mut tab, "a", "ArrowRight"), "b");
    assert_eq!(from(&mut tab, "b", "ArrowRight"), "c");
    assert_eq!(from(&mut tab, "c", "ArrowRight"), "c");
    assert_eq!(from(&mut tab, "c", "ArrowLeft"), "b");
    assert_eq!(from(&mut tab, "a", "ArrowDown"), "a");
    // A menu's block axis, wrapping.
    assert_eq!(from(&mut tab, "m1", "ArrowDown"), "m2");
    assert_eq!(from(&mut tab, "m2", "ArrowDown"), "m1");
    assert_eq!(from(&mut tab, "m1", "ArrowRight"), "m1");
    // A negative `tabindex` is no target, but moves from.
    assert_eq!(from(&mut tab, "w1", "ArrowRight"), "w3");
    assert_eq!(from(&mut tab, "w2", "ArrowLeft"), "w1");
    assert_eq!(from(&mut tab, "w3", "ArrowDown"), "w1");
    // Home and End, whatever the axis.
    assert_eq!(from(&mut tab, "c", "Home"), "a");
    assert_eq!(from(&mut tab, "a", "End"), "c");
    assert_eq!(from(&mut tab, "a", "Home"), "a");
}

#[test]
fn opting_out_nesting_and_the_top_layer_bound_a_focusgroup() {
    let mut tab = page(
        "<div focusgroup=toolbar><button id=a>a</button>\
         <span focusgroup=none><button id=out>out</button></span>\
         <div id=inner focusgroup=toolbar tabindex=0><button id=i1>1</button>\
         <button id=i2>2</button></div>\
         <div id=pop popover><button id=p>p</button></div><button id=b>b</button></div>\
         <div dir=rtl focusgroup=toolbar><button id=r1>1</button><button id=r2>2</button></div>",
    );
    assert_eq!(from(&mut tab, "a", "ArrowRight"), "inner");
    assert_eq!(from(&mut tab, "inner", "ArrowRight"), "b");
    assert_eq!(from(&mut tab, "i1", "ArrowRight"), "i2");
    assert_eq!(from(&mut tab, "out", "ArrowRight"), "out");
    // A popover shown is out of the focusgroup around it.
    crate::differential::run_script(&mut tab.engine, "pop.showPopover()");
    assert_eq!(from(&mut tab, "b", "ArrowLeft"), "inner");
    assert_eq!(from(&mut tab, "p", "ArrowLeft"), "p");
    // Right to left, Left goes on.
    assert_eq!(from(&mut tab, "r1", "ArrowLeft"), "r2");
    assert_eq!(from(&mut tab, "r2", "ArrowRight"), "r1");
}

#[test]
fn elements_that_take_the_arrows_keep_them() {
    let mut tab = page(
        "<div focusgroup='toolbar inline block'><button id=a>a</button>\
         <input id=t value=text><select id=s><option>1</option></select>\
         <div id=e contenteditable tabindex=0>e</div><button id=b>b</button></div>",
    );
    assert_eq!(from(&mut tab, "a", "ArrowRight"), "t");
    assert_eq!(from(&mut tab, "t", "ArrowDown"), "t");
    assert_eq!(from(&mut tab, "s", "ArrowRight"), "s");
    assert_eq!(from(&mut tab, "e", "ArrowRight"), "e");
    assert_eq!(from(&mut tab, "t", "End"), "t");
}

#[test]
fn a_grid_moves_by_rows_and_columns_with_their_spans() {
    let mut tab = page(
        "<table focusgroup='grid rowwrap colflow'>\
         <tr><td id=a tabindex=0>a</td><td id=b tabindex=0 colspan=2>b</td></tr>\
         <tr><td id=c tabindex=0>c</td><td id=d>d</td><td id=e tabindex=0>e</td></tr>\
         </table><div focusgroup=grid><span id=n tabindex=0>n</span>\
         <span id=m tabindex=0>m</span></div>",
    );
    assert_eq!(from(&mut tab, "a", "ArrowRight"), "b");
    // Past the row's end, its start.
    assert_eq!(from(&mut tab, "b", "ArrowRight"), "a");
    // Past what cannot take the focus.
    assert_eq!(from(&mut tab, "c", "ArrowRight"), "e");
    assert_eq!(from(&mut tab, "e", "ArrowUp"), "b");
    assert_eq!(from(&mut tab, "a", "ArrowDown"), "c");
    // Past the column's end, the next column's start.
    assert_eq!(from(&mut tab, "c", "ArrowDown"), "b");
    // Not a table: no grid, and no list either.
    assert_eq!(from(&mut tab, "n", "ArrowRight"), "n");
}

#[test]
fn tab_stops_once_in_each_segment_at_its_entry() {
    let mut tab = page(
        "<button id=before>before</button>\
         <div focusgroup=toolbar><button id=a>a</button><button id=b focusgroupstart>b</button>\
         <span focusgroup=none><button id=out>out</button></span>\
         <button id=c>c</button><input id=t><button id=d>d</button></div>\
         <div focusgroup='toolbar nomemory'><button id=n1>1</button>\
         <button id=n2>2</button></div><button id=after>after</button>",
    );
    assert_eq!(
        tabs(&mut tab, "before", false),
        ["b", "out", "c", "n1", "after"]
    );
    // Memory: the item focused last is the entry.
    focus(&mut tab, "n2");
    focus(&mut tab, "d");
    assert_eq!(
        tabs(&mut tab, "after", true),
        ["n1", "d", "out", "b", "before"]
    );
    // An element that takes the arrows goes on to the item beside it.
    focus(&mut tab, "t");
    press(&mut tab, "Tab", false);
    assert_eq!(focused(&tab), "d");
    focus(&mut tab, "t");
    press(&mut tab, "Tab", true);
    assert_eq!(focused(&tab), "c");
}

#[test]
fn tab_goes_from_where_the_focused_element_left_the_tree() {
    let mut tab = page(
        "<button id=one>one</button><div id=box><button id=gone>gone</button></div>\
         <button id=two>two</button><button id=three>three</button>",
    );
    focus(&mut tab, "gone");
    crate::differential::run_script(&mut tab.engine, "box.remove()");
    assert_eq!(focused(&tab), "-");
    press(&mut tab, "Tab", false);
    assert_eq!(focused(&tab), "two");
    // A blurred element stays the starting point.
    focus(&mut tab, "two");
    crate::differential::run_script(&mut tab.engine, "document.getElementById('two').blur()");
    press(&mut tab, "Tab", true);
    assert_eq!(focused(&tab), "one");
}

#[test]
fn a_showing_popover_is_in_its_invokers_scope() {
    let mut tab = page(
        "<button id=first>first</button><button id=inv>inv</button>\
         <button id=last>last</button><div id=pop popover><button id=in>in</button></div>",
    );
    crate::differential::run_script(&mut tab.engine, "pop.showPopover({source: inv})");
    assert_eq!(tabs(&mut tab, "first", false), ["inv", "in", "last"]);
    assert_eq!(tabs(&mut tab, "last", true), ["in", "inv", "first"]);
}

#[test]
fn the_arrows_move_through_a_radio_group_and_check() {
    let mut tab = page(
        "<input type=radio name=r id=r1 checked><input type=radio name=r id=r2>\
         <input type=radio name=r id=r3 disabled><button id=b>b</button>",
    );
    assert_eq!(from(&mut tab, "r1", "ArrowRight"), "r2");
    let document = tab.engine.document();
    assert!(dom::forms::checkedness(document, by_id(&tab.engine, "r2")));
    assert_eq!(from(&mut tab, "r2", "ArrowDown"), "r1");
    // Tab passes the group's unchecked buttons.
    assert_eq!(tabs(&mut tab, "r1", false), ["b"]);
}

#[test]
fn autofocus_takes_the_first_candidate_inserted() {
    let mut tab = page(
        "<input autofocus id=i1><input autofocus id=i2>\
         <script>const a = document.getElementById('i1');\
         a.remove(); document.body.append(a);</script>",
    );
    render(&mut tab);
    assert_eq!(focused(&tab), "i2");
    // Processed: a later one does not take it.
    crate::differential::run_script(
        &mut tab.engine,
        "const late = document.createElement('input'); late.id = 'late'; \
         late.autofocus = true; document.body.append(late); \
         document.activeElement.blur();",
    );
    render(&mut tab);
    assert_eq!(focused(&tab), "-");
}

#[test]
fn focus_visible_is_as_focus_options_say() {
    let mut tab = page("<button id=b>b</button><input id=t>");
    let visible = |tab: &Tab, id: &str| {
        tab.engine
            .document()
            .state(by_id(&tab.engine, id))
            .contains(ElementState::FOCUS_VISIBLE)
    };
    crate::differential::run_script(&mut tab.engine, "b.focus()");
    assert!(!visible(&tab, "b"));
    crate::differential::run_script(&mut tab.engine, "b.blur(); b.focus({focusVisible: true})");
    assert!(visible(&tab, "b"));
    crate::differential::run_script(&mut tab.engine, "t.focus({focusVisible: false})");
    assert!(!visible(&tab, "t"));
    // A text field shows it unless told not to.
    crate::differential::run_script(&mut tab.engine, "t.blur(); t.focus()");
    assert!(visible(&tab, "t"));
}

#[test]
fn content_editable_reflects_its_attribute_and_the_fixup_follows_it() {
    let mut tab = page("<div id=e contenteditable>e</div><p id=p>p</p>");
    crate::differential::run_script(
        &mut tab.engine,
        "console.log(e.contentEditable, e.isContentEditable, p.isContentEditable); \
         e.focus(); e.contentEditable = 'false'; \
         console.log(e.getAttribute('contenteditable'), e.isContentEditable); \
         try { e.contentEditable = 'maybe'; } catch (x) { console.log(x.name); }",
    );
    assert_eq!(
        lines(&mut tab),
        ["true true false", "false false", "SyntaxError"]
    );
    // The focus fixup at the rendering update takes the focus.
    assert_eq!(focused(&tab), "e");
    render(&mut tab);
    assert_eq!(focused(&tab), "-");
}

#[test]
fn focus_group_reflects_its_tokens() {
    let mut tab = page("<div id=g focusgroup='toolbar wrap'></div>");
    crate::differential::run_script(
        &mut tab.engine,
        "console.log(g.focusGroup instanceof DOMTokenList, g.focusGroup === g.focusGroup, \
         g.focusGroup.supports('grid'), g.focusGroup.supports('row-wrap'), g.focusGroupStart); \
         g.focusGroup = 'menu'; g.focusGroupStart = true; \
         console.log(g.getAttribute('focusgroup'), g.hasAttribute('focusgroupstart'));",
    );
    assert_eq!(lines(&mut tab), ["true true true false false", "menu true"]);
}

#[test]
fn svg_text_and_intersection_questions_are_answered() {
    let mut tab = page(
        "<svg id=s width=200 height=200><path id=p d='M5 5 L10 5 L10 10 L5 10 Z' \
         style='stroke: green; stroke-width: 4px'/><text id=t x=10 y=50 font-size=20>abc</text>\
         <rect id=r x=100 y=100 width=10 height=10 style='pointer-events: none'/></svg>",
    );
    crate::differential::run_script(
        &mut tab.engine,
        "const q = (x, y, w, h) => { const r = s.createSVGRect(); \
           r.x = x; r.y = y; r.width = w; r.height = h; return r; }; \
         console.log(t.getNumberOfChars(), t.getCharNumAtPosition(new DOMPoint(12, 45)), \
           t.getCharNumAtPosition(new DOMPoint(0, 0)), t.getExtentOfChar(1).width > 0, \
           Math.abs(t.getStartPositionOfChar(1).x - t.getEndPositionOfChar(0).x) < 1e-6); \
         try { t.getStartPositionOfChar(3); } catch (e) { console.log(e.name); } \
         console.log(s.checkIntersection(p, q(11, 11, 5, 5)), \
           s.checkIntersection(p, q(13, 13, 5, 5)), s.checkEnclosure(p, q(0, 0, 50, 50)), \
           s.checkEnclosure(p, q(5, 5, 5, 5)), \
           Array.from(s.getIntersectionList(q(0, 0, 200, 200), null)).map(e => e.id).join()); \
         const m = p.getScreenCTM(); console.log(m.e, m.f, p.ownerSVGElement === s);",
    );
    assert_eq!(
        lines(&mut tab),
        [
            "3 0 -1 true true",
            "IndexSizeError",
            "true false true false p,t",
            "8 8 true"
        ]
    );
}

#[test]
fn observers_pass_over_skipped_contents() {
    let mut tab = page(
        "<div id=box><div id=target style='width: 50px; height: 50px'></div></div>\
         <script>let sizes = 0, seen = [];\
         new ResizeObserver(() => sizes++).observe(target);\
         new IntersectionObserver(es => es.forEach(e => seen.push(e.isIntersecting)))\
         .observe(target);</script>",
    );
    render(&mut tab);
    render(&mut tab);
    crate::differential::run_script(
        &mut tab.engine,
        "box.style.contentVisibility = 'hidden'; target.style.width = '80px'; sizes = 0;",
    );
    render(&mut tab);
    render(&mut tab);
    crate::differential::run_script(&mut tab.engine, "console.log(sizes, seen.join())");
    assert_eq!(lines(&mut tab), ["0 true,false"]);
    crate::differential::run_script(&mut tab.engine, "box.style.contentVisibility = 'visible'");
    render(&mut tab);
    render(&mut tab);
    crate::differential::run_script(&mut tab.engine, "console.log(sizes, seen.join())");
    assert_eq!(lines(&mut tab), ["1 true,false,true"]);
}

#[test]
fn a_frame_whose_script_focuses_takes_the_focus_around_it() {
    let mut tab = page(
        "<iframe id=f srcdoc=\"<input id=i><script>document.getElementById('i').focus()</script>\">\
         </iframe>",
    );
    for _ in 0..4 {
        tab.engine.run_tasks();
        tab.engine.poll_network();
        render(&mut tab);
    }
    assert_eq!(focused(&tab), "f");
    let (mut tab, console) = scripted(
        "<script>addEventListener('load', () => console.log('at load', \
         document.activeElement.tagName))</script>\
         <iframe srcdoc=\"<input><script>document.querySelector('input').focus();</script>\">\
         </iframe>",
        true,
    );
    assert_eq!(console, ["at load IFRAME"]);
    for _ in 0..4 {
        tab.engine.run_tasks();
        tab.engine.poll_network();
        render(&mut tab);
    }
    crate::differential::run_script(
        &mut tab.engine,
        "console.log(document.activeElement.tagName)",
    );
    assert_eq!(lines(&mut tab), ["IFRAME"]);
}
