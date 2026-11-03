//! The top layer, dialogs, popovers and invokers (gap batch H): their
//! scripts' API and events in the reference browser's order, the top
//! layer's inertness, hit testing and focus, the user's Escape, light
//! dismiss, invoker buttons and interest invokers, with scripts on and,
//! by the engine's own algorithms, off.

use dom::{ElementState, NodeId};
use net::test_server::TestServer;

use super::tests::load_all;
use crate::testing::*;
use crate::{Button, Key, KeyOutcome};

/// `body` served and loaded with scripts on or off and a clock the test
/// moves, a 400 by 300 frame drawn: the tab, the clock and the server.
fn page(body: &str, scripts: bool) -> (Tab, HandClock, TestServer) {
    let body: &'static str = Box::leak(body.to_owned().into_boxed_str());
    let server = serve(move |path| match path {
        "/page.html" => Some(html(body)),
        _ => None,
    });
    let mut tab = Tab::new();
    let clock = tab.hand_clock();
    tab.engine.set_scripting(scripts);
    load_all(&mut tab, server.url("/page.html"));
    tab.engine.frame(400, 300);
    lines(&mut tab);
    (tab, clock, server)
}

/// The console's lines since they were last taken.
fn lines(tab: &mut Tab) -> Vec<String> {
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

/// Runs `script` in the page as a task of its own, then the tasks it
/// queued; the lines logged.
fn run(tab: &mut Tab, clock: &HandClock, script: &str) -> Vec<String> {
    tab.engine.evaluate_script(script);
    clock.advance(1);
    tab.engine.run_tasks();
    render(tab, clock);
    lines(tab)
}

/// A frame drawn with its rendering steps at the clock's time.
fn render(tab: &mut Tab, clock: &HandClock) {
    clock.advance(16);
    let now = tab.engine.clock_now();
    tab.engine.frame_at(
        400,
        300,
        crate::FrameTime {
            now,
            deadline: now + std::time::Duration::from_millis(16),
        },
    );
}

#[test]
fn a_scroller_focused_keeps_the_focus_and_takes_the_keys() {
    // A scroll container that can scroll takes the focus (gap batch H):
    // the frames' focus fixup, by the styles and the layout as they now
    // are, leaves it focused, and an arrow key scrolls it.
    let (mut tab, clock, _server) = page(
        "<div id=s style='width:200px;height:200px;overflow-y:scroll'>\
         <div style='height:600px'></div></div>",
        true,
    );
    render(&mut tab, &clock);
    let focused = run(
        &mut tab,
        &clock,
        "s.focus(); console.log(document.activeElement === s)",
    );
    assert_eq!(focused, ["true"]);
    let s = id(&tab, "s");
    tab.engine.key_down(&Key::new("ArrowDown", "ArrowDown"));
    for _ in 0..30 {
        render(&mut tab, &clock);
    }
    assert_eq!(tab.engine.document().focused(), Some(s));
    assert!(tab.engine.element_scroll(s) > 0.0);
}

#[test]
fn a_popover_that_is_a_shadow_host_keeps_its_slotted_content_in_order() {
    // Shown by its invoker, the popover is navigated after it; its own
    // tree's elements are its scope's, and what its slot takes is the
    // slot's, as for any host (HTML section 6.6.3, gap batch H).
    let (mut tab, clock, _server) = page(
        "<button id=invoker popovertarget=popover>Open</button>\
         <x-pop id=popover><input type=checkbox id=one><input type=checkbox id=two></x-pop>\
         <button id=last>Last</button>\
         <script>customElements.define('x-pop', class extends HTMLElement {\
           constructor() { super(); this.attachShadow({mode: 'open'});\
             this.setAttribute('popover', 'auto');\
             this.shadowRoot.innerHTML = '<button id=a>a</button><slot></slot><button id=b>b</button>'; }\
         });</script>",
        true,
    );
    render(&mut tab, &clock);
    run(&mut tab, &clock, "invoker.focus()");
    tab.engine.key_down(&Key::new("Enter", "Enter"));
    tab.engine.key_up(&Key::new("Enter", "Enter"));
    render(&mut tab, &clock);
    let probe = "{const a = document.activeElement; \
                 console.log(a.shadowRoot && a.shadowRoot.activeElement \
                   ? a.id + '/' + a.shadowRoot.activeElement.id : a.id)}";
    let mut seen = Vec::new();
    for _ in 0..5 {
        tab.engine.focus_next(false);
        render(&mut tab, &clock);
        seen.extend(run(&mut tab, &clock, probe));
    }
    assert_eq!(seen, ["popover/a", "one", "two", "popover/b", "last"]);
}

fn id(tab: &Tab, id: &str) -> NodeId {
    by_id(&tab.engine, id)
}

fn state(tab: &Tab, name: &str) -> ElementState {
    tab.engine.document().state(id(tab, name))
}

/// The ID of the element focused, or `-`.
fn focused(tab: &Tab) -> String {
    let document = tab.engine.document();
    tab.engine.focused().map_or("-".to_owned(), |node| {
        document.attr(node, "id").unwrap_or("?").to_owned()
    })
}

fn click(tab: &mut Tab, x: f32, y: f32) {
    tab.engine.pointer_move(x, y);
    tab.engine.pointer_down(x, y, Button::Primary, 1);
    tab.engine.pointer_up(x, y, Button::Primary, 1);
}

fn escape(tab: &mut Tab) -> KeyOutcome {
    let key = Key::new("Escape", "Escape");
    let outcome = tab.engine.key_down(&key);
    tab.engine.key_up(&key);
    outcome
}

/// Logs the top layer's events at each element of `ids`.
fn logger(ids: &[&str]) -> String {
    let list: Vec<String> = ids.iter().map(|i| format!("'{i}'")).collect();
    format!(
        "<script>[{}].forEach(function (i) {{ var e = document.getElementById(i);\
         ['beforetoggle','toggle','cancel','close','command','interest','loseinterest']\
         .forEach(function (t) {{ e.addEventListener(t, function (ev) {{\
           console.log(t + '@' + i + (ev.oldState ? ' ' + ev.oldState + '>' + ev.newState : '') +\
             (ev.command ? ' ' + ev.command : '') + (ev.source ? ' by ' + ev.source.id : '') +\
             (ev.cancelable ? ' c' : '')); }}); }}); }});</script>",
        list.join(",")
    )
}

const DIALOG: &str = "<style>body { margin: 0 }</style><input id=prev>\
    <dialog id=d><p>text</p><input id=i1><input id=i2 autofocus></dialog>\
    <dialog id=d2><p>nothing</p></dialog>";

#[test]
fn show_modal_puts_the_dialog_in_the_top_layer_and_close_gives_the_focus_back() {
    let (mut tab, clock, _server) = page(&format!("{DIALOG}{}", logger(&["d"])), true);
    let lines = run(
        &mut tab,
        &clock,
        "document.getElementById('prev').focus(); var d = document.getElementById('d');\
         d.showModal(); console.log('open', d.open, d.matches(':modal'), d.matches(':open'), \
         document.activeElement.id, d.closedBy);\
         try { d.show(); } catch (e) { console.log(e.name); }\
         d.showModal(); console.log('again');",
    );
    assert_eq!(
        lines,
        [
            "beforetoggle@d closed>open c",
            "open true true true i2 closerequest",
            "InvalidStateError",
            "again",
            "toggle@d closed>open",
        ]
    );
    assert!(state(&tab, "d").contains(ElementState::MODAL));
    let d = id(&tab, "d");
    assert_eq!(tab.engine.document().top_layer(), [d]);
    // Everything else is inert: the page's input takes no focus.
    let lines = run(
        &mut tab,
        &clock,
        "document.getElementById('prev').focus(); console.log(document.activeElement.id);\
         var d = document.getElementById('d'); d.close('done');\
         console.log(d.open, d.returnValue, document.activeElement.id, d.matches(':modal'));\
         d.close('other'); console.log(d.returnValue);",
    );
    assert_eq!(
        lines,
        [
            "i2",
            "beforetoggle@d open>closed",
            "false done prev false",
            "done",
            "toggle@d open>closed",
            "close@d",
        ]
    );
    assert!(tab.engine.document().top_layer().is_empty());
}

#[test]
fn a_dialog_with_nothing_to_focus_takes_the_focus_itself() {
    let (mut tab, clock, _server) = page(DIALOG, true);
    let lines = run(
        &mut tab,
        &clock,
        "var d = document.getElementById('d2'); d.show();\
         console.log(document.activeElement.id, d.matches(':modal'));\
         d.close(); d.returnValue = 'kept'; d.showModal(); d.close();\
         console.log(d.returnValue, document.activeElement.id || document.activeElement.tagName);",
    );
    assert_eq!(lines, ["d2 false", "kept BODY"]);
}

#[test]
fn toggle_events_of_a_run_of_changes_are_one() {
    let (mut tab, clock, _server) = page(
        &format!("<div popover id=p>p</div>{}", logger(&["p"])),
        true,
    );
    let lines = run(
        &mut tab,
        &clock,
        "var p = document.getElementById('p'); p.showPopover(); p.hidePopover(); p.showPopover();\
         console.log('sync', p.matches(':popover-open'));",
    );
    assert_eq!(
        lines,
        [
            "beforetoggle@p closed>open c",
            "beforetoggle@p open>closed",
            "beforetoggle@p closed>open c",
            "sync true",
            "toggle@p closed>open",
        ]
    );
}

#[test]
fn the_popover_api_checks_validity_as_the_reference_browser_does() {
    let (mut tab, clock, _server) = page("<div popover id=p>p</div><div id=n>n</div>", true);
    let lines = run(
        &mut tab,
        &clock,
        "var p = document.getElementById('p');\
         try { document.getElementById('n').showPopover(); } catch (e) { console.log(e.name); }\
         var x = document.createElement('div'); x.popover = 'auto';\
         try { x.showPopover(); } catch (e) { console.log(e.name); }\
         x.hidePopover(); console.log('hidden ok');\
         console.log(p.togglePopover(), p.togglePopover(), p.togglePopover({force: false}), \
           p.togglePopover(true));\
         p.showPopover(); console.log('again ok');\
         console.log(p.popover, x.popover, document.getElementById('n').popover);\
         x.setAttribute('popover', 'Hint'); console.log(x.popover);\
         x.setAttribute('popover', 'bogus'); console.log(x.popover);",
    );
    assert_eq!(
        lines,
        [
            "NotSupportedError",
            "InvalidStateError",
            "hidden ok",
            "true false false true",
            "again ok",
            "auto auto null",
            "hint",
            "manual",
        ]
    );
}

#[test]
fn auto_popovers_stack_by_ancestry_and_the_invoker() {
    let (mut tab, clock, _server) = page(
        "<div popover id=p1>p1<div popover id=p2>p2</div><button id=b>b</button></div>\
         <div popover id=p3>p3</div><div popover=manual id=m>m</div>",
        true,
    );
    let open = "console.log(Array.from(document.querySelectorAll(':popover-open'))\
        .map(function (e) { return e.id; }).join());";
    let lines = run(
        &mut tab,
        &clock,
        &format!(
            "var $ = function (i) {{ return document.getElementById(i); }};\
             $('m').showPopover(); $('p1').showPopover(); $('p2').showPopover(); {open}\
             $('p3').showPopover(); {open}\
             $('p1').showPopover(); $('p3').showPopover({{source: $('b')}}); {open}\
             $('p1').hidePopover(); {open}"
        ),
    );
    assert_eq!(lines, ["p1,p2,m", "p3,m", "p1,p3,m", "m"]);
}

#[test]
fn hint_popovers_stack_as_the_reference_browsers_do() {
    let (mut tab, clock, _server) = page(
        "<div popover id=p1>p1<div popover id=p2>p2</div><div popover=hint id=hp>hp</div></div>\
         <div popover id=p3>p3</div><div popover=hint id=hx>hx</div>\
         <div popover=hint id=h2>h2<div popover id=a3>a3</div><div popover=hint id=h3>h3</div></div>",
        true,
    );
    let script = "var $ = function (i) { return document.getElementById(i); };\
        var reset = function () { var x; while ((x = document.querySelector(':popover-open'))) \
          x.hidePopover(); };\
        var run = function (name, ids, hide) { reset(); ids.forEach(function (i) { \
          $(i).showPopover(); }); if (hide) $(hide).hidePopover(); \
          console.log(name, Array.from(document.querySelectorAll(':popover-open'))\
            .map(function (e) { return e.id; }).join()); };\
        run('S1', ['p1', 'h2'], 'h2'); run('S2', ['p1', 'h2', 'h3']);\
        run('S4', ['p1', 'h2', 'p3']); run('S5', ['p1', 'h2', 'p2']);\
        run('S6', ['p1', 'h2', 'a3']); run('S6d', ['h2', 'a3'], 'a3');\
        run('S7', ['p1', 'hx', 'h2']); run('S8', ['p1', 'hp', 'hx']);\
        run('S10', ['p1', 'hp'], 'p1'); run('S11', ['p1', 'p2', 'hp']);\
        run('S12', ['hx', 'p1']); run('S13', ['p1', 'hx'], 'p1');";
    assert_eq!(
        run(&mut tab, &clock, script),
        [
            "S1 p1",
            "S2 h2,h3",
            "S4 p3",
            "S5 p1,p2",
            "S6 h2,a3",
            "S6d h2",
            "S7 p1,h2",
            "S8 p1,hx",
            "S10 ",
            "S11 p1,p2,hp",
            "S12 p1",
            "S13 hx",
        ]
    );
}

#[test]
fn a_popover_is_hidden_when_its_attribute_changes_or_it_leaves_the_document() {
    let (mut tab, clock, _server) = page(
        &format!("<div popover id=p>p</div>{}", logger(&["p"])),
        true,
    );
    let lines = run(
        &mut tab,
        &clock,
        "var p = document.getElementById('p'); p.showPopover(); p.popover = 'manual';\
         console.log(p.matches(':popover-open'));\
         p.showPopover(); p.remove(); console.log(p.matches(':popover-open'));",
    );
    assert_eq!(
        lines,
        [
            "beforetoggle@p closed>open c",
            "beforetoggle@p open>closed",
            "false",
            "beforetoggle@p closed>open c",
            "false",
            "toggle@p closed>open",
        ]
    );
    assert!(tab.engine.document().top_layer().is_empty());
}

#[test]
fn show_modal_hides_the_popovers_it_is_not_in() {
    let (mut tab, clock, _server) = page(
        "<div popover id=p>p<dialog id=in>in</dialog></div><div popover id=q>q</div>\
         <dialog id=out>out</dialog>",
        true,
    );
    let lines = run(
        &mut tab,
        &clock,
        "var $ = function (i) { return document.getElementById(i); };\
         $('p').showPopover(); $('in').showModal();\
         console.log($('p').matches(':popover-open'));\
         $('in').close(); $('out').showModal(); console.log($('p').matches(':popover-open'));",
    );
    assert_eq!(lines, ["true", "false"]);
}

#[test]
fn a_command_button_runs_its_command_after_a_cancelable_event() {
    let (mut tab, clock, _server) = page(
        &format!(
            "<button id=sm commandfor=d command=show-modal>m</button>\
             <dialog id=d><button id=cl commandfor=d command=close value=bv>c</button>\
             <button id=rc commandfor=d command=request-close>r</button></dialog>\
             <button id=cu commandfor=t command=--Go>c</button><div id=t>t</div>\
             <button id=tp commandfor=p command=toggle-popover>p</button>\
             <div popover id=p>p</div>{}",
            logger(&["d", "t", "p"])
        ),
        true,
    );
    let lines = run(
        &mut tab,
        &clock,
        "var $ = function (i) { return document.getElementById(i); };\
         console.log($('sm').command, $('cu').command, $('sm').commandForElement.id);\
         $('sm').click(); console.log($('d').matches(':modal'));\
         $('cl').click(); console.log($('d').open, $('d').returnValue);\
         $('cu').click();\
         $('t').addEventListener('command', function (e) { e.preventDefault(); });\
         $('tp').click(); console.log($('p').matches(':popover-open'));\
         $('sm').command = 'bogus'; console.log(JSON.stringify($('sm').command));",
    );
    assert_eq!(
        lines,
        [
            "show-modal --Go d",
            "command@d show-modal by sm c",
            "beforetoggle@d closed>open by sm c",
            "true",
            "command@d close by cl c",
            "beforetoggle@d open>closed by cl",
            "false bv",
            "command@t --Go by cu c",
            "command@p toggle-popover by tp c",
            "beforetoggle@p closed>open by tp c",
            "true",
            "\"\"",
            "toggle@d closed>closed by cl",
            "toggle@p closed>open by tp",
            "close@d",
        ]
    );
}

#[test]
fn a_popover_target_button_toggles_its_popover_as_its_action_says() {
    let (mut tab, clock, _server) = page(
        "<style>body { margin: 0 } button { display: block; width: 100px; height: 30px }</style>\
         <button id=t popovertarget=p>t</button>\
         <button id=s popovertarget=p popovertargetaction=show>s</button>\
         <div popover id=p>p</div>",
        true,
    );
    // By the user's press, with scripts on.
    click(&mut tab, 10.0, 10.0);
    assert!(state(&tab, "p").contains(ElementState::POPOVER_OPEN));
    click(&mut tab, 10.0, 40.0);
    assert!(state(&tab, "p").contains(ElementState::POPOVER_OPEN));
    click(&mut tab, 10.0, 10.0);
    assert!(!state(&tab, "p").contains(ElementState::POPOVER_OPEN));
    let lines = run(
        &mut tab,
        &clock,
        "var $ = function (i) { return document.getElementById(i); };\
         console.log($('t').popoverTargetElement.id, $('t').popoverTargetAction, \
           $('s').popoverTargetAction);\
         $('t').popoverTargetElement = null; console.log($('t').hasAttribute('popovertarget'));\
         $('t').popoverTargetElement = $('p'); console.log(JSON.stringify(\
           $('t').getAttribute('popovertarget')), $('t').popoverTargetElement.id);",
    );
    assert_eq!(lines, ["p toggle show", "false", "\"\" p"]);
}

#[test]
fn escape_closes_the_topmost_dialog_or_popover_as_its_close_watcher() {
    let (mut tab, clock, _server) = page(
        &format!(
            "{DIALOG}<div popover id=p>p</div><dialog id=n closedby=none>n</dialog>{}",
            logger(&["d", "p"])
        ),
        true,
    );
    // Shown in one task, with no activation between, they are one group
    // of close watchers: one Escape closes both, the newest first, the
    // dialog after a `cancel` no listener canceled.
    run(
        &mut tab,
        &clock,
        "document.getElementById('d').showModal(); document.getElementById('p').showPopover();",
    );
    assert_eq!(escape(&mut tab), KeyOutcome::Taken);
    assert!(!state(&tab, "p").contains(ElementState::POPOVER_OPEN));
    assert!(!state(&tab, "d").contains(ElementState::MODAL));
    render(&mut tab, &clock);
    let logged = lines(&mut tab);
    assert!(logged.contains(&"cancel@d c".to_owned()) || logged.contains(&"cancel@d".to_owned()));
    let hid = logged
        .iter()
        .position(|l| l == "beforetoggle@p open>closed");
    let closed = logged
        .iter()
        .position(|l| l == "beforetoggle@d open>closed");
    assert!(hid < closed, "{logged:?}");
    // The user's activation between makes the popover a group of its
    // own: an Escape closes it alone, the next the dialog.
    run(
        &mut tab,
        &clock,
        "document.getElementById('d').showModal();",
    );
    click(&mut tab, 5.0, 5.0);
    run(
        &mut tab,
        &clock,
        "document.getElementById('p').showPopover();",
    );
    assert_eq!(escape(&mut tab), KeyOutcome::Taken);
    assert!(!state(&tab, "p").contains(ElementState::POPOVER_OPEN));
    assert!(state(&tab, "d").contains(ElementState::MODAL));
    escape(&mut tab);
    assert!(!state(&tab, "d").contains(ElementState::MODAL));
    // A dialog closed by none has its watcher disabled: the Escape goes
    // on, unprocessed.
    run(
        &mut tab,
        &clock,
        "document.getElementById('n').showModal();",
    );
    assert_eq!(escape(&mut tab), KeyOutcome::Ignored);
    assert!(state(&tab, "n").contains(ElementState::MODAL));
}

#[test]
fn close_comes_with_the_next_frame() {
    let (mut tab, clock, _server) = page(&format!("{DIALOG}{}", logger(&["d"])), true);
    tab.engine
        .evaluate_script("var d = document.getElementById('d'); d.showModal(); d.close();");
    clock.advance(5);
    tab.engine.run_tasks();
    // Its tasks run, the toggle among them; not `close`.
    assert_eq!(
        lines(&mut tab),
        [
            "beforetoggle@d closed>open c",
            "beforetoggle@d open>closed",
            "toggle@d closed>closed"
        ]
    );
    render(&mut tab, &clock);
    assert_eq!(lines(&mut tab), ["close@d"]);
}

#[test]
fn a_cancel_is_cancelable_only_after_the_users_activation() {
    let (mut tab, clock, _server) = page(
        &format!(
            "{DIALOG}<script>document.getElementById('d').addEventListener('cancel', \
             function (e) {{ e.preventDefault(); }});</script>{}",
            logger(&["d"])
        ),
        true,
    );
    run(
        &mut tab,
        &clock,
        "document.getElementById('d').showModal();",
    );
    // No activation since the page loaded: the cancel cannot keep it.
    escape(&mut tab);
    assert!(!state(&tab, "d").contains(ElementState::MODAL));
    run(
        &mut tab,
        &clock,
        "document.getElementById('d').showModal();",
    );
    click(&mut tab, 5.0, 5.0);
    lines(&mut tab);
    escape(&mut tab);
    assert!(state(&tab, "d").contains(ElementState::MODAL));
    // Used up: the next Escape closes it.
    escape(&mut tab);
    assert!(!state(&tab, "d").contains(ElementState::MODAL));
}

#[test]
fn a_press_outside_light_dismisses_auto_popovers_and_closedby_any_dialogs() {
    let (mut tab, clock, _server) = page(
        "<style>body { margin: 0 } [popover], dialog { margin: 0; inset: auto; left: 200px; \
         top: 0; width: 100px; height: 50px; padding: 0; border: 0 }</style>\
         <div popover id=p>p</div><div popover=manual id=m style='top: 100px'>m</div>\
         <dialog id=d closedby=any>d</dialog>",
        true,
    );
    run(
        &mut tab,
        &clock,
        "document.getElementById('p').showPopover(); document.getElementById('m').showPopover();",
    );
    tab.engine.frame(400, 300);
    // Inside it stays; outside it goes, a manual one stays.
    click(&mut tab, 210.0, 10.0);
    assert!(state(&tab, "p").contains(ElementState::POPOVER_OPEN));
    click(&mut tab, 10.0, 200.0);
    assert!(!state(&tab, "p").contains(ElementState::POPOVER_OPEN));
    assert!(state(&tab, "m").contains(ElementState::POPOVER_OPEN));
    run(
        &mut tab,
        &clock,
        "document.getElementById('m').hidePopover(); document.getElementById('d').showModal();",
    );
    tab.engine.frame(400, 300);
    click(&mut tab, 210.0, 10.0);
    assert!(state(&tab, "d").contains(ElementState::MODAL));
    // The backdrop is the dialog's, outside its box.
    click(&mut tab, 10.0, 200.0);
    assert!(!state(&tab, "d").contains(ElementState::MODAL));
}

#[test]
fn a_modal_dialog_blocks_the_pointer_and_a_popovers_backdrop_lets_it_through() {
    let (mut tab, clock, _server) = page(
        "<style>body { margin: 0 } #under { width: 400px; height: 300px }</style>\
         <div id=under>under</div><dialog id=d>d</dialog><div popover id=p>p</div>",
        true,
    );
    run(
        &mut tab,
        &clock,
        "document.getElementById('p').showPopover();",
    );
    tab.engine.frame(400, 300);
    let lines = run(
        &mut tab,
        &clock,
        "console.log(document.elementFromPoint(5, 5).id);\
         document.getElementById('p').hidePopover(); document.getElementById('d').showModal();\
         console.log(document.elementFromPoint(5, 5).id);",
    );
    assert_eq!(lines, ["under", "d"]);
}

#[test]
fn with_scripts_off_the_engine_runs_the_invokers_escape_and_light_dismiss() {
    let (mut tab, _clock, _server) = page(
        "<style>body { margin: 0 } button { display: block; width: 100px; height: 30px }\
         [popover], dialog { margin: 0; inset: auto; left: 200px; top: 0; width: 100px; \
         height: 50px; padding: 0; border: 0 }</style>\
         <button id=t popovertarget=p>t</button>\
         <button id=m commandfor=d command=show-modal>m</button>\
         <div popover id=p>p<input id=in autofocus></div>\
         <dialog id=d><form method=dialog><button id=ok>ok</button></form></dialog>",
        false,
    );
    click(&mut tab, 10.0, 10.0);
    assert!(state(&tab, "p").contains(ElementState::POPOVER_OPEN));
    assert_eq!(focused(&tab), "in");
    escape(&mut tab);
    assert!(!state(&tab, "p").contains(ElementState::POPOVER_OPEN));
    assert_eq!(focused(&tab), "t");
    click(&mut tab, 10.0, 10.0);
    tab.engine.frame(400, 300);
    click(&mut tab, 10.0, 200.0);
    assert!(!state(&tab, "p").contains(ElementState::POPOVER_OPEN));
    click(&mut tab, 10.0, 40.0);
    assert!(state(&tab, "d").contains(ElementState::MODAL));
    assert_eq!(focused(&tab), "ok");
    tab.engine.frame(400, 300);
    // The dialog method closes it, the focus back on the button.
    click(&mut tab, 210.0, 10.0);
    assert!(!state(&tab, "d").contains(ElementState::MODAL));
    assert!(tab.engine.document().attr(id(&tab, "d"), "open").is_none());
    assert_eq!(focused(&tab), "m");
}

#[test]
fn a_form_of_the_dialog_method_closes_its_dialog_with_the_submitters_value() {
    let (mut tab, clock, _server) = page(
        &format!(
            "<dialog id=d><form method=dialog><button id=ok value=yes>ok</button></form></dialog>{}",
            logger(&["d"])
        ),
        true,
    );
    let lines = run(
        &mut tab,
        &clock,
        "var d = document.getElementById('d'); d.showModal();\
         document.getElementById('ok').click(); console.log(d.open, d.returnValue);",
    );
    assert_eq!(
        lines,
        [
            "beforetoggle@d closed>open c",
            "beforetoggle@d open>closed",
            "false yes",
            "toggle@d closed>closed",
            "close@d",
        ]
    );
}

#[test]
fn interest_follows_the_focus_after_the_delays() {
    let (mut tab, clock, _server) = page(
        &format!(
            "<button id=b interestfor=t>b</button><div popover=hint id=t>t</div>\
             <input id=o>{}",
            logger(&["t"])
        ),
        true,
    );
    run(&mut tab, &clock, "document.getElementById('b').focus();");
    assert!(!state(&tab, "t").contains(ElementState::POPOVER_OPEN));
    clock.advance(600);
    tab.engine.run_tasks();
    assert!(state(&tab, "t").contains(ElementState::POPOVER_OPEN));
    assert!(state(&tab, "b").contains(ElementState::INTEREST_SOURCE));
    assert!(state(&tab, "t").contains(ElementState::INTEREST_TARGET));
    let logged = lines(&mut tab);
    assert_eq!(logged[0], "interest@t by b c");
    run(&mut tab, &clock, "document.getElementById('o').focus();");
    clock.advance(300);
    tab.engine.run_tasks();
    assert!(!state(&tab, "t").contains(ElementState::POPOVER_OPEN));
    assert!(!state(&tab, "b").contains(ElementState::INTEREST_SOURCE));
    assert!(lines(&mut tab).contains(&"loseinterest@t by b c".to_owned()));
    let lines = run(
        &mut tab,
        &clock,
        "var b = document.getElementById('b'); console.log(b.interestForElement.id,\
         typeof InterestEvent, new InterestEvent('interest', {source: b}).source.id,\
         new ToggleEvent('toggle', {oldState: 'a'}).oldState,\
         new CommandEvent('command', {command: '--x'}).command);",
    );
    assert_eq!(lines, ["t function b a --x"]);
}

#[test]
fn a_close_watcher_hears_escape_and_its_own_requests() {
    let (mut tab, clock, _server) = page("<p>x</p>", true);
    let lines = run(
        &mut tab,
        &clock,
        "window.w = new CloseWatcher(); w.oncancel = function (e) { console.log('cancel', \
         e.cancelable); }; w.onclose = function () { console.log('close'); };\
         var v = new CloseWatcher(); v.addEventListener('close', function () { \
         console.log('v close'); }); v.requestClose(); v.close();",
    );
    // Asked by the page, with no listener canceling, it closes, once.
    assert_eq!(lines, ["v close"]);
    // The user's Escape, with no activation: a cancel it cannot cancel.
    assert_eq!(escape(&mut tab), KeyOutcome::Taken);
    assert_eq!(
        super::top_layer_tests::lines(&mut tab),
        ["cancel false", "close"]
    );
    // Destroyed, it hears no more.
    assert_eq!(escape(&mut tab), KeyOutcome::Ignored);
}

#[test]
fn a_drop_downs_list_shown_is_open() {
    let (mut tab, clock, _server) = page(
        "<style>body { margin: 0 } select { display: block; width: 100px }</style>\
         <select id=s><option>a</option><option>b</option></select>\
         <details id=dt open><summary>s</summary>x</details>",
        true,
    );
    click(&mut tab, 10.0, 5.0);
    assert!(state(&tab, "s").contains(ElementState::OPEN));
    let lines = run(
        &mut tab,
        &clock,
        "console.log(document.getElementById('s').matches(':open'), \
         document.getElementById('dt').matches(':open'));",
    );
    assert_eq!(lines, ["true true"]);
    escape(&mut tab);
    assert!(!state(&tab, "s").contains(ElementState::OPEN));
}

/// The node of ID `id` in the shadow tree of the host of ID `host`.
fn inner(tab: &Tab, host: &str, id: &str) -> NodeId {
    let document = tab.engine.document();
    let root = document
        .shadow_root(by_id(&tab.engine, host))
        .expect("a shadow root");
    document.element_by_id_in(root, id).expect("the element")
}

const SHADOWED: &str = "<style>body { margin: 0 } button { display: block; width: 100px; \
    height: 30px }</style>\
    <div id=host><template shadowrootmode=open>\
      <button id=b popovertarget=p>b</button>\
      <div popover id=p>p<input id=pi autofocus></div>\
      <dialog id=d><input id=di></dialog>\
      <button id=m commandfor=d command=show-modal>m</button>\
    </template></div>\
    <button id=out commandfor=ref command=--go>out</button>\
    <button id=pt popovertarget=ref2>pt</button>\
    <label id=l for=ref3>label</label>\
    <div id=ref><template shadowrootmode=open shadowrootreferencetarget=t>\
      <div id=t>t</div></template></div>\
    <div id=ref2><template shadowrootmode=open shadowrootreferencetarget=q>\
      <div popover id=q>q</div></template></div>\
    <div id=ref3><template shadowrootmode=open shadowrootreferencetarget=i>\
      <input id=i></template></div>";

#[test]
fn dialogs_and_popovers_in_a_shadow_tree_reach_the_top_layer() {
    let (mut tab, clock, _server) = page(SHADOWED, true);
    // A user's click on a button in the shadow tree shows its popover,
    // the page having no realm yet: the engine's own algorithms,
    // which takes the focus to its `autofocus` input.
    click(&mut tab, 10.0, 10.0);
    let p = inner(&tab, "host", "p");
    assert!(
        tab.engine
            .document()
            .state(p)
            .contains(ElementState::POPOVER_OPEN)
    );
    assert_eq!(tab.engine.document().top_layer(), [p]);
    let lines = run(
        &mut tab,
        &clock,
        "var r = document.getElementById('host').shadowRoot;\
         console.log(r.activeElement.id, document.activeElement.id);\
         r.getElementById('m').click();\
         console.log(r.getElementById('d').matches(':modal'), r.activeElement.id, \
           r.getElementById('p').matches(':popover-open'));",
    );
    assert_eq!(lines, ["pi host", "true di false"]);
    let d = inner(&tab, "host", "d");
    assert_eq!(tab.engine.document().top_layer(), [d]);
    // Its box is at the top level, centred in the viewport.
    let lines = run(
        &mut tab,
        &clock,
        "var d = document.getElementById('host').shadowRoot.getElementById('d');\
         var r = d.getBoundingClientRect(); console.log(r.width > 0, \
         document.elementFromPoint(2, 2).id); d.close();",
    );
    assert_eq!(lines, ["true host"]);
    // Taking the host out takes the dialog's state with it.
    run(
        &mut tab,
        &clock,
        "var r = document.getElementById('host').shadowRoot; \
         r.getElementById('d').showModal(); document.getElementById('host').remove();",
    );
    assert!(tab.engine.document().top_layer().is_empty());
}

#[test]
fn invokers_follow_reference_targets_into_shadow_trees() {
    let (mut tab, clock, _server) = page(SHADOWED, true);
    let lines = run(
        &mut tab,
        &clock,
        "var $ = function (i) { return document.getElementById(i); };\
         var t = $('ref').shadowRoot.getElementById('t');\
         t.addEventListener('command', function (e) { console.log('at t', e.target.id, \
           e.source.id, e.composedPath().length); });\
         $('ref').addEventListener('command', function (e) { console.log('at ref', \
           e.target.id); });\
         console.log($('out').commandForElement.id, $('pt').popoverTargetElement.id, \
           $('l').control.id);\
         $('out').click(); $('pt').click();\
         console.log($('ref2').shadowRoot.getElementById('q').matches(':popover-open'));\
         $('l').click(); console.log(document.activeElement.id, \
           $('ref3').shadowRoot.activeElement.id, \
           $('ref3').shadowRoot.getElementById('i').labels.length);\
         $('ref').shadowRoot.referenceTarget = 'gone'; $('out').click(); console.log('none');",
    );
    assert_eq!(
        lines,
        [
            "ref ref2 ref3",
            "at t t out 7",
            "at ref ref",
            "true",
            "ref3 i 1",
            "none",
        ]
    );
}

#[test]
fn a_scripts_elements_and_aria_references_are_reflected() {
    let (mut tab, clock, _server) = page(
        "<button id=b commandfor=x command=--go>b</button><div id=x>x</div><div id=y>y</div>\
         <div id=a aria-labelledby='x y'></div>",
        true,
    );
    let lines = run(
        &mut tab,
        &clock,
        "var $ = function (i) { return document.getElementById(i); };\
         $('y').addEventListener('command', function () { console.log('at y'); });\
         $('b').commandForElement = $('y');\
         console.log(JSON.stringify($('b').getAttribute('commandfor')), \
           $('b').commandForElement.id); $('b').click();\
         $('b').setAttribute('commandfor', 'x'); console.log($('b').commandForElement.id);\
         $('b').commandForElement = $('y'); $('b').setAttribute('commandfor', '');\
         console.log($('b').commandForElement); $('b').setAttribute('commandfor', 'x');\
         console.log($('a').ariaLabelledByElements.map(function (e) { return e.id; }).join());\
         $('a').ariaLabelledByElements = [$('y')]; console.log(JSON.stringify(\
           $('a').getAttribute('aria-labelledby')), $('a').ariaLabelledByElements[0].id);\
         $('a').ariaLabelledByElements = null; console.log($('a').ariaLabelledByElements, \
           $('a').hasAttribute('aria-labelledby'), $('a').ariaActiveDescendantElement);",
    );
    assert_eq!(
        lines,
        [
            "\"\" y",
            "at y",
            "x",
            "null",
            "x,y",
            "\"\" y",
            "null false null"
        ]
    );
}

#[test]
fn a_popover_in_a_shadow_tree_nests_by_the_flat_tree_and_stays_on_a_press_in_it() {
    let (mut tab, clock, _server) = page(
        "<style>body { margin: 0 } [popover] { margin: 0; inset: auto; left: 0; top: 0; \
         width: 200px; height: 100px; padding: 0; border: 0 }</style>\
         <div popover id=outer><div id=host><template shadowrootmode=open>\
         <div><div popover id=innerp style='margin: 0; inset: auto; left: 0; top: 0; width: 50px; height: 50px; padding: 0; border: 0'>i</div></div>\
         </template></div></div>",
        true,
    );
    let lines = run(
        &mut tab,
        &clock,
        "document.getElementById('outer').showPopover();\
         var i = document.getElementById('host').shadowRoot.getElementById('innerp');\
         i.showPopover(); console.log(document.getElementById('outer').matches(':popover-open'), \
         i.matches(':popover-open'));",
    );
    assert_eq!(lines, ["true true"]);
    // A press in the inner popover keeps both; one outside both closes
    // both.
    click(&mut tab, 10.0, 10.0);
    let inner_popover = inner(&tab, "host", "innerp");
    assert!(
        tab.engine
            .document()
            .state(inner_popover)
            .contains(ElementState::POPOVER_OPEN)
    );
    assert!(state(&tab, "outer").contains(ElementState::POPOVER_OPEN));
    click(&mut tab, 300.0, 250.0);
    assert!(!state(&tab, "outer").contains(ElementState::POPOVER_OPEN));
}

#[test]
fn a_form_attribute_follows_reference_targets_and_submit_stays_in_the_submitters_tree() {
    let (mut tab, clock, _server) = page(
        "<div id=h><template shadowrootmode=open><button id=b form=x>b</button>\
         <input id=i form=x name=z value=1>\
         <div id=x><template shadowrootmode=open shadowrootreferencetarget=f><form id=f></form>\
         </template></div></template></div>",
        true,
    );
    let lines = run(
        &mut tab,
        &clock,
        "var r = document.getElementById('h').shadowRoot; var b = r.getElementById('b');\
         var x = r.getElementById('x'); var f = x.shadowRoot.getElementById('f');\
         console.log(b.form.id, r.getElementById('i').form.id, f.elements.length, \
           f.elements.z === r.getElementById('i'), Array.from(new FormData(f).keys()).join());\
         f.addEventListener('submit', function (e) { e.preventDefault(); console.log('f', \
           e.composed, e.submitter && e.submitter.id, e.composedPath().length); });\
         x.addEventListener('submit', function (e) { console.log('x', e.target.id); });\
         document.getElementById('h').addEventListener('submit', function (e) { \
           console.log('h', e.target.id); });\
         window.addEventListener('submit', function (e) { console.log('window'); }, true);\
         b.click(); f.requestSubmit();",
    );
    assert_eq!(
        lines,
        ["x x 2 true z", "f true b 4", "x x", "f false null 2"]
    );
}

#[test]
fn details_toggle_open_by_their_attribute_their_summary_and_their_name() {
    let (mut tab, clock, _server) = page(
        "<style>body { margin: 0 } summary { display: block; height: 20px }</style>\
         <details id=a name=g><summary id=s>s</summary>a</details>\
         <details id=b name=g open>b</details>\
         <details id=c ontoggle=\"console.log('parsed', event.oldState, event.newState)\" open>c</details>\
         <script>['a', 'b'].forEach(function (i) { document.getElementById(i)\
           .addEventListener('toggle', function (e) { console.log('toggle', i, e.oldState, \
           e.newState, e.bubbles, e.cancelable, e.source, e instanceof ToggleEvent); }); });\
         </script>",
        true,
    );
    // The parser's `open` owes its toggle too.
    let lines = run(
        &mut tab,
        &clock,
        "console.log(document.getElementById('a').open);",
    );
    assert_eq!(
        lines,
        [
            "false",
            "toggle b closed open false false null true",
            "parsed closed open"
        ]
    );
    // Opening one of a name closes the other; opened and closed again
    // before its task, one toggle from closed to closed, its task the
    // last queued.
    let lines = run(
        &mut tab,
        &clock,
        "var a = document.getElementById('a'); a.open = true;\
         console.log(document.getElementById('b').open, a.name);\
         a.open = false; a.open = true; a.removeAttribute('open');",
    );
    assert_eq!(
        lines,
        [
            "false g",
            "toggle b open closed false false null true",
            "toggle a closed closed false false null true",
        ]
    );
    // Its summary opens it, by a click or by the keyboard, and takes the
    // focus.
    click(&mut tab, 10.0, 10.0);
    let lines = run(&mut tab, &clock, "console.log(document.activeElement.id);");
    assert_eq!(lines, ["s", "toggle a closed open false false null true"]);
    assert!(tab.engine.document().attr(id(&tab, "a"), "open").is_some());
    let enter = Key::new("Enter", "Enter");
    tab.engine.key_down(&enter);
    tab.engine.key_up(&enter);
    let lines = run(&mut tab, &clock, "");
    assert_eq!(lines, ["toggle a open closed false false null true"]);
    // A slot changed and a `details` toggled in one script: each taken
    // once, as each native call tells of the other still waiting.
    let lines = run(
        &mut tab,
        &clock,
        "var h = document.createElement('div'); var r = h.attachShadow({mode: 'open'});\
         var s = document.createElement('slot'); r.appendChild(s);\
         s.addEventListener('slotchange', function () { console.log('slotchange'); });\
         h.appendChild(document.createElement('span')); document.getElementById('c').open = false;",
    );
    assert_eq!(lines, ["slotchange", "parsed open closed"]);
}

#[test]
fn a_button_with_a_command_is_no_submit_button_and_show_picker_wants_the_users_activation() {
    let (mut tab, clock, _server) = page(
        "<style>body { margin: 0 } button { display: block; height: 20px }</style>\
         <form id=f><button id=a commandfor=p>a</button><button id=b type=bogus command=--x>b</button>\
         <button id=c>c</button></form><div popover id=p>p</div>\
         <select id=s><option>1</option><option>2</option></select>\
         <select id=d disabled><option>1</option></select>",
        true,
    );
    let lines = run(
        &mut tab,
        &clock,
        "var f = document.getElementById('f'); f.addEventListener('submit', function (e) {\
           e.preventDefault(); console.log('submit'); });\
         console.log(['a', 'b', 'c'].map(function (i) { return document.getElementById(i).type; }).join());\
         document.getElementById('b').click(); document.getElementById('c').click();\
         try { document.getElementById('s').showPicker(); } catch (e) { console.log(e.name); }\
         try { document.getElementById('d').showPicker(); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(
        lines,
        [
            "button,button,submit",
            "submit",
            "NotAllowedError",
            "InvalidStateError"
        ]
    );
    // With the user's activation its list shows, and it matches `:open`.
    click(&mut tab, 300.0, 250.0);
    let lines = run(
        &mut tab,
        &clock,
        "var s = document.getElementById('s'); s.showPicker(); console.log('shown');",
    );
    assert_eq!(lines, ["shown"]);
    assert_eq!(tab.engine.popup_open(), Some(id(&tab, "s")));
    assert!(state(&tab, "s").contains(ElementState::OPEN));
}

#[test]
fn a_dialog_shown_as_a_popover_too_toggles_as_each() {
    // Its popover's `toggle` and its dialog's are tracked apart (HTML's
    // two toggle task trackers): one of each.
    let (mut tab, clock, _server) = page(
        &format!("<dialog id=d popover>d</dialog>{}", logger(&["d"])),
        true,
    );
    let lines = run(
        &mut tab,
        &clock,
        "var d = document.getElementById('d');\
         d.addEventListener('beforetoggle', function () { d.showPopover(); }, {once: true});\
         d.show(); console.log('open', d.open);",
    );
    assert_eq!(
        lines,
        [
            "beforetoggle@d closed>open c",
            "beforetoggle@d closed>open c",
            "beforetoggle@d open>closed",
            "open true",
            "toggle@d closed>open",
            "toggle@d closed>closed",
        ]
    );
}

#[test]
fn a_dialogs_requests_and_its_forms_submit_close_it_as_the_reference_browser_does() {
    let (mut tab, clock, _server) = page(
        "<dialog id=d><form id=f method=dialog><button id=b>b</button></form></dialog>",
        true,
    );
    let lines = run(
        &mut tab,
        &clock,
        "var d = document.getElementById('d'); d.returnValue = 'kept'; d.showModal();\
         document.getElementById('f').submit(); console.log(d.open, JSON.stringify(d.returnValue));\
         var other = document.implementation.createHTMLDocument('');\
         var o = other.body.appendChild(other.createElement('dialog')); o.setAttribute('open', '');\
         o.requestClose(); console.log(o.open);",
    );
    // `submit()` has no submitter: the result is the empty string; a
    // dialog of a document that is not fully active stays open.
    assert_eq!(lines, ["false \"\"", "true"]);
}

#[test]
fn the_dialog_and_popover_focusing_steps_stop_the_documents_autofocus() {
    let (mut tab, clock, _server) = page(
        "<dialog id=d><input id=a><input id=b autofocus></dialog>",
        true,
    );
    let lines = run(
        &mut tab,
        &clock,
        "document.getElementById('d').show(); console.log(document.activeElement.id);\
         document.activeElement.blur();\
         var late = document.body.appendChild(document.createElement('input'));\
         late.id = 'late'; late.autofocus = true;",
    );
    assert_eq!(lines, ["b"]);
    render(&mut tab, &clock);
    render(&mut tab, &clock);
    assert_eq!(focused(&tab), "-");
}

#[test]
fn a_modal_dialog_inert_or_making_the_focus_inert_leaves_it_to_the_viewport() {
    let (mut tab, clock, _server) = page(
        "<style>body { margin: 0 } dialog { width: 100%; height: 100%; max-width: 100%; \
         max-height: 100%; box-sizing: border-box; padding: 0 } \
         dialog::backdrop { display: none }</style>\
         <input id=outer><dialog id=d><input id=inner autofocus></dialog>",
        true,
    );
    let lines = run(
        &mut tab,
        &clock,
        "var d = document.getElementById('d'); d.showModal();\
         console.log(document.elementFromPoint(10, 10).id);\
         d.inert = true; console.log(document.elementFromPoint(10, 10).tagName);\
         d.close(); document.getElementById('outer').focus(); d.showModal();\
         console.log(document.activeElement.tagName);",
    );
    // An `inert` attribute restyles at once; the focus on what the modal
    // dialog made inert, the dialog inert too, goes to the viewport.
    assert_eq!(lines, ["inner", "HTML", "BODY"]);
}

#[test]
fn a_press_on_what_is_inert_or_past_every_box_is_the_roots() {
    let (mut tab, clock, _server) = page(
        "<style>body { margin: 0 } #box { width: 100px; height: 100px } dialog { top: 200px; \
         margin: 0; left: 200px } dialog::backdrop { display: none }</style>\
         <div id=box></div><dialog id=d>d</dialog>\
         <script>['mousedown', 'mouseup', 'click'].forEach(function (t) {\
           document.addEventListener(t, function (e) { console.log(t, e.target.tagName); }); });\
         </script>",
        true,
    );
    run(
        &mut tab,
        &clock,
        "document.getElementById('d').showModal();",
    );
    tab.engine.frame(400, 300);
    click(&mut tab, 50.0, 50.0);
    assert_eq!(
        lines(&mut tab),
        ["mousedown HTML", "mouseup HTML", "click HTML"]
    );
}

#[test]
fn a_dialog_opened_by_its_attribute_has_a_close_watcher() {
    let (mut tab, clock, _server) = page(
        &format!(
            "<dialog id=d open closedby=any>d</dialog>{}",
            logger(&["d"])
        ),
        true,
    );
    assert!(tab.engine.document().dialog(id(&tab, "d")).is_some());
    escape(&mut tab);
    let lines = run(
        &mut tab,
        &clock,
        "console.log(document.getElementById('d').open)",
    );
    assert_eq!(
        lines,
        [
            "cancel@d",
            "beforetoggle@d open>closed",
            "false",
            "toggle@d open>closed",
            "close@d",
        ]
    );
    // The attribute set by script and taken away by it again.
    run(
        &mut tab,
        &clock,
        "document.getElementById('d').setAttribute('open', '');",
    );
    assert!(tab.engine.document().dialog(id(&tab, "d")).is_some());
    run(
        &mut tab,
        &clock,
        "document.getElementById('d').removeAttribute('open');",
    );
    assert!(tab.engine.document().dialog(id(&tab, "d")).is_none());
}

#[test]
fn an_editing_host_takes_the_focus_and_tab_reaches_it() {
    let (mut tab, clock, _server) = page(
        "<button id=a>a</button><div id=e contenteditable>e<span contenteditable=true>\
         s</span></div><div id=f contenteditable=false>f</div><dialog id=d open \
         contenteditable>d</dialog>",
        true,
    );
    let lines = run(
        &mut tab,
        &clock,
        "document.getElementById('e').focus(); console.log(document.activeElement.id);\
         document.getElementById('f').focus(); console.log(document.activeElement.id);\
         console.log(document.getElementById('e').tabIndex);",
    );
    assert_eq!(lines, ["e", "e", "0"]);
    let tab_key = Key::new("Tab", "Tab");
    run(&mut tab, &clock, "document.getElementById('a').focus();");
    tab.engine.key_down(&tab_key);
    assert_eq!(focused(&tab), "e");
    tab.engine.key_down(&tab_key);
    assert_eq!(focused(&tab), "d");
}

#[test]
fn a_dialog_popover_and_a_hosts_popover_focus_as_the_reference_browser_does() {
    let (mut tab, clock, _server) = page(
        "<dialog id=d popover autofocus><button id=inside>b</button></dialog>\
         <div id=h popover tabindex=0><template shadowrootmode=open>\
         <button autofocus>x</button></template></div>",
        true,
    );
    let lines = run(
        &mut tab,
        &clock,
        "var d = document.getElementById('d'); d.showPopover();\
         console.log(document.activeElement.id); d.hidePopover();\
         d.removeAttribute('autofocus'); d.showPopover(); console.log(document.activeElement.id);\
         d.focus(); console.log(document.activeElement.id); d.hidePopover();\
         document.getElementById('h').showPopover(); console.log(document.activeElement.tagName);",
    );
    // With `autofocus` the dialog itself; else its first focusable area;
    // a shown dialog takes `focus()`; a host's shadow tree is not looked
    // in for an `autofocus` element.
    assert_eq!(lines, ["d", "inside", "d", "BODY"]);
}

#[test]
fn no_popover_is_shown_while_another_is_being_shown_or_hidden() {
    let (mut tab, clock, _server) = page(
        "<div popover id=a>a</div><div popover id=b>b</div><div popover=manual id=m>m</div>",
        true,
    );
    let lines = run(
        &mut tab,
        &clock,
        "var a = document.getElementById('a'), b = document.getElementById('b'),\
           m = document.getElementById('m');\
         function tryShow(p) { try { p.showPopover(); return 'shown'; } \
           catch (e) { return e.name + ': ' + e.message; } }\
         a.addEventListener('beforetoggle', function (e) { console.log(e.newState, tryShow(m)); });\
         a.showPopover(); a.hidePopover(); console.log(m.matches(':popover-open'), tryShow(b));",
    );
    let refused = "InvalidStateError: Failed to execute 'showPopover' on 'HTMLElement': \
                   Invalid to show a popover during another show operation";
    assert_eq!(
        lines,
        [
            format!("open {refused}"),
            format!("closed {refused}"),
            "false shown".to_owned(),
        ]
    );
}

/// A page of an auto popover holding another and a hint, and an
/// unrelated auto, each a box of its own, for presses.
const STACKED: &str = "<style>body { margin: 0 } [popover] { margin: 0; padding: 0; border: 0; \
    inset: auto; width: 50px; height: 50px; left: 0 } #a { top: 0 } #a2 { top: 60px } \
    #h { top: 120px } #u { top: 180px }</style>\
    <div popover id=a><div popover id=a2>a2</div><div popover=hint id=h>h</div></div>\
    <div popover=hint id=u>u</div>";

#[test]
fn a_press_in_a_hint_or_an_auto_closes_what_is_above_it_in_the_stack() {
    // With scripts, the bindings' light dismiss, a realm made by a
    // script; without, the engine's.
    for scripts in [true, false] {
        let (mut tab, _clock, _server) =
            page(&format!("{STACKED}<script>var realm = 1</script>"), scripts);
        let open = |tab: &Tab, name: &str| state(tab, name).contains(ElementState::POPOVER_OPEN);
        let show = |tab: &mut Tab| {
            for name in ["a", "a2", "h"] {
                let node = id(tab, name);
                tab.engine.show_popover(node, None);
            }
            tab.engine.frame(400, 300);
        };
        show(&mut tab);
        assert!(open(&tab, "a") && open(&tab, "a2") && open(&tab, "h"));
        // In the hint: the auto above the one the hint hangs from goes.
        click(&mut tab, 10.0, 130.0);
        assert!(
            open(&tab, "a") && !open(&tab, "a2") && open(&tab, "h"),
            "{scripts}"
        );
        // In an auto: every hint goes.
        click(&mut tab, 10.0, 10.0);
        assert!(open(&tab, "a") && !open(&tab, "h"), "{scripts}");
        // In a hint that hangs from no auto popover: every auto goes.
        let u = id(&tab, "u");
        tab.engine.show_popover(u, None);
        tab.engine.frame(400, 300);
        assert!(open(&tab, "a") && open(&tab, "u"), "{scripts}");
        click(&mut tab, 10.0, 190.0);
        assert!(!open(&tab, "a") && open(&tab, "u"), "{scripts}");
    }
}

#[test]
fn a_modal_dialog_in_an_auto_popover_closes_the_hints_too() {
    let (mut tab, clock, _server) = page(&format!("{STACKED}<dialog id=d>d</dialog>"), true);
    let lines = run(
        &mut tab,
        &clock,
        "['a', 'a2', 'h'].forEach(function (i) { document.getElementById(i).showPopover(); });\
         var d = document.getElementById('d'); document.getElementById('a').appendChild(d);\
         d.showModal();\
         console.log(['a', 'a2', 'h'].map(function (i) {\
           return document.getElementById(i).matches(':popover-open'); }).join());\
         d.close(); var a = document.getElementById('a');\
         var open = document.body.appendChild(document.createElement('dialog'));\
         open.popover = 'manual'; open.setAttribute('open', ''); open.showPopover();\
         console.log(open.matches(':popover-open'));",
    );
    // A non-modal dialog takes `showPopover()` too.
    assert_eq!(lines, ["true,false,false", "true"]);
}

#[test]
fn a_labels_focus_and_a_slotted_elements_focus_go_where_the_reference_browser_has_them() {
    let (mut tab, clock, _server) = page(
        "<label id=l for=c>label</label><input id=c><button id=prev>p</button>\
         <div id=host><template shadowrootmode=open><dialog id=sd><slot></slot></dialog>\
         </template><button id=slotted>s</button></div>",
        true,
    );
    let lines = run(
        &mut tab,
        &clock,
        "document.getElementById('l').focus(); console.log(document.activeElement.id);\
         var d = document.getElementById('host').shadowRoot.getElementById('sd');\
         document.getElementById('prev').focus(); d.show();\
         document.getElementById('slotted').focus(); d.close();\
         console.log(document.activeElement.id);",
    );
    assert_eq!(lines, ["c", "prev"]);
}

#[test]
fn a_click_on_a_link_with_no_href_is_its_summarys_and_an_svg_link_is_followed() {
    let (mut tab, clock, _server) = page(
        "<details id=d><summary><a id=a><i id=i>open</i></a></summary>x</details>\
         <svg width=100 height=100><a href='#there'><circle id=c cx=50 cy=50 r=40 /></a></svg>",
        true,
    );
    let lines = run(
        &mut tab,
        &clock,
        "document.getElementById('i').click(); console.log(document.getElementById('d').open);\
         document.getElementById('c').dispatchEvent(new MouseEvent('click', {bubbles: true, \
         cancelable: true}));",
    );
    assert_eq!(lines, ["true"]);
    assert_eq!(tab.engine.url().and_then(net::Url::fragment), Some("there"));
}

#[test]
fn a_light_dismiss_comes_after_the_mouseup_and_hover_stops_at_the_top_layer() {
    let (mut tab, clock, _server) = page(
        "<style>body { margin: 0 }</style><div id=host><template shadowrootmode=open>\
         <style>[popover] { margin: 0; padding: 0; border: 0; inset: auto; left: 200px; \
         top: 0; width: 100px; height: 50px }</style><div popover id=p><slot></slot></div>\
         </template><span id=s>s</span></div>\
         <script>['pointerup', 'mouseup', 'click'].forEach(function (t) {\
           document.addEventListener(t, function () { console.log(t); }); });</script>",
        true,
    );
    let shown = run(
        &mut tab,
        &clock,
        "var p = document.getElementById('host').shadowRoot.getElementById('p');\
         p.addEventListener('beforetoggle', function (e) { console.log(e.newState); });\
         p.showPopover();",
    );
    assert_eq!(shown, ["open"]);
    tab.engine.frame(400, 300);
    tab.engine.pointer_move(202.0, 5.0);
    assert!(state(&tab, "s").contains(ElementState::HOVER));
    assert!(!state(&tab, "host").contains(ElementState::HOVER));
    click(&mut tab, 10.0, 200.0);
    assert_eq!(lines(&mut tab), ["pointerup", "mouseup", "closed", "click"]);
}

#[test]
fn tab_and_the_keys_go_on_from_where_a_press_left_the_focus() {
    let (mut tab, clock, _server) = page(
        "<style>body { margin: 0 } #s { height: 100px; overflow-y: scroll } \
         #s div { height: 1000px }</style>\
         <button id=a>a</button><div id=s><div><p id=text>text</p></div></div>\
         <button id=b>b</button>",
        true,
    );
    tab.engine.frame(400, 300);
    run(&mut tab, &clock, "document.getElementById('a').focus();");
    // A press on the text takes the focus from the button to the viewport.
    click(&mut tab, 10.0, 40.0);
    assert_eq!(focused(&tab), "-");
    let s = id(&tab, "s");
    tab.engine.key_down(&Key::new("PageDown", "PageDown"));
    assert!(tab.engine.element_scroll_target(s) > 0.0);
    assert_eq!(tab.engine.scroll_target(), 0.0);
    tab.engine.key_down(&Key::new("Tab", "Tab"));
    assert_eq!(focused(&tab), "b");
}

#[test]
fn a_precise_wheel_scrolls_at_once() {
    let (mut tab, _clock, _server) = page(
        "<style>body { margin: 0 } #s { height: 100px; overflow-y: scroll } \
         #s div { height: 1000px }</style><div id=s><div></div></div>",
        true,
    );
    tab.engine.frame(400, 300);
    let s = id(&tab, "s");
    tab.engine.wheel_precise(10.0, 10.0, 50.0);
    assert_eq!(tab.engine.element_scroll(s), 50.0);
    tab.engine.wheel(10.0, 10.0, 50.0);
    assert_eq!(tab.engine.element_scroll(s), 50.0);
    assert_eq!(tab.engine.element_scroll_target(s), 100.0);
}
