//! Constraint validation (M26.9b, `docs/design/events.md`): each
//! validity state and its message, a custom validity, candidates, the
//! lengths after the user's edit, `checkValidity()` and
//! `reportValidity()`, interactive validation stopping a submission, and
//! the pseudo-classes of form state.

use net::test_server::TestServer;

use super::script_tests::scripted;
use super::tests::load_all;
use crate::testing::*;
use crate::{Key, LoadState};

/// The console's lines since they were last taken.
fn lines(tab: &mut Tab) -> Vec<String> {
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

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

/// Logs each element's failing states, whether it will validate, and
/// its message, as `id: states | will | message`.
const REPORT: &str = "function report(id) { var e = document.getElementById(id); var v = e.validity; \
    var states = ['valueMissing','typeMismatch','patternMismatch','tooLong','tooShort', \
      'rangeUnderflow','rangeOverflow','stepMismatch','customError'].filter(function (s) { return v[s]; }); \
    console.log(id + ': ' + states.join(',') + ' | ' + e.willValidate + ' ' + v.valid + ' | ' + e.validationMessage); }";

#[test]
fn each_state_has_the_reference_browsers_message() {
    let mut tab = page(
        "<input id=req required><input id=em type=email value=nope>\
         <input id=em2 type=email multiple value='a@b.c, x@'>\
         <input id=url type=url value='not a url'><input id=pat pattern='[0-9]{3}' value=12a>\
         <input id=under type=number min=2 max=10 value=1>\
         <input id=over type=number max=10 value=11><input id=step type=number min=1 step=3 value=5>\
         <input id=cb type=checkbox required>\
         <input type=radio name=g id=r1 required><input type=radio name=g id=r2>\
         <select id=sel required><option value=''>Pick</option><option>a</option></select>\
         <textarea id=ta required></textarea><input id=dis required disabled>\
         <input id=ro required readonly><input id=ok value=x>",
    );
    run(&mut tab, REPORT);
    let ids = [
        "req", "em", "em2", "url", "pat", "under", "over", "step", "cb", "r2", "sel", "ta", "dis",
        "ro", "ok",
    ];
    let source: String = ids.iter().map(|id| format!("report('{id}');")).collect();
    assert_eq!(
        run(&mut tab, &source),
        [
            "req: valueMissing | true false | Please fill out this field.",
            "em: typeMismatch | true false | Please include an '@' in the email address. 'nope' is missing an '@'.",
            "em2: typeMismatch | true false | Please enter an email address.",
            "url: typeMismatch | true false | Please enter a URL.",
            "pat: patternMismatch | true false | Please match the requested format.",
            "under: rangeUnderflow | true false | Value must be greater than or equal to 2.",
            "over: rangeOverflow | true false | Value must be less than or equal to 10.",
            "step: stepMismatch | true false | Please enter a valid value. The two nearest valid values are 4 and 7.",
            "cb: valueMissing | true false | Please check this box if you want to proceed.",
            "r2: valueMissing | true false | Please select one of these options.",
            "sel: valueMissing | true false | Please select an item in the list.",
            "ta: valueMissing | true false | Please fill out this field.",
            "dis: valueMissing | false false | ",
            "ro: valueMissing | false false | ",
            "ok:  | true true | ",
        ]
    );
    // Filled in, they pass.
    assert_eq!(
        run(
            &mut tab,
            "document.getElementById('req').value = 'x'; document.getElementById('cb').checked = true; \
             document.getElementById('r1').checked = true; document.getElementById('sel').selectedIndex = 1; \
             document.getElementById('pat').value = '123'; report('req'); report('cb'); report('r2'); \
             report('sel'); report('pat');"
        ),
        [
            "req:  | true true | ",
            "cb:  | true true | ",
            "r2:  | true true | ",
            "sel:  | true true | ",
            "pat:  | true true | ",
        ]
    );
    // A custom validity comes first, and goes when emptied.
    assert_eq!(
        run(
            &mut tab,
            "var ok = document.getElementById('ok'); ok.setCustomValidity('Nope'); report('ok'); \
             ok.setCustomValidity(''); report('ok');"
        ),
        ["ok: customError | true false | Nope", "ok:  | true true | "]
    );
}

#[test]
fn lengths_count_after_the_users_edit() {
    let mut tab =
        page("<input id=long maxlength=3 value=abcdef><input id=short minlength=5 value=a>");
    run(&mut tab, REPORT);
    assert_eq!(
        run(&mut tab, "report('long'); report('short');"),
        ["long:  | true true | ", "short:  | true true | "],
        "a value the user did not edit"
    );
    run(&mut tab, "document.getElementById('long').focus();");
    tab.engine.key_down(&Key::new("End", "End"));
    tab.engine.key_down(&Key::new("Backspace", "Backspace"));
    run(&mut tab, "document.getElementById('short').focus();");
    tab.engine.key_down(&Key::new("End", "End"));
    tab.engine.key_down(&Key::new("b", "KeyB").with_text("b"));
    assert_eq!(
        run(&mut tab, "report('long'); report('short');"),
        [
            "long: tooLong | true false | Please shorten this text to 3 characters or less (you are currently using 5 characters).",
            "short: tooShort | true false | Please lengthen this text to 5 characters or more (you are currently using 2 characters).",
        ]
    );
}

#[test]
fn check_and_report_validity_fire_invalid() {
    let mut tab = page(
        "<input id=a required><input id=b><form id=f><input id=c required><input id=d required></form>\
         <script>var stop = false; ['a','c','d'].forEach(function (id) { \
           document.getElementById(id).addEventListener('invalid', function (e) { \
             console.log('invalid ' + id + ' ' + e.cancelable + ' ' + e.bubbles); \
             if (stop) e.preventDefault(); }); });</script>",
    );
    assert_eq!(
        run(
            &mut tab,
            "console.log(document.getElementById('a').checkValidity(), \
               document.getElementById('b').checkValidity());"
        ),
        ["invalid a true false", "false true"]
    );
    assert_eq!(tab.engine.focused(), None);
    // reportValidity: the event, then the focus and the notice.
    assert_eq!(
        run(
            &mut tab,
            "console.log(document.getElementById('a').reportValidity());"
        ),
        ["invalid a true false", "false"]
    );
    let a = by_id(&tab.engine, "a");
    assert_eq!(tab.engine.focused(), Some(a));
    assert_eq!(
        tab.engine.take_notice().as_deref(),
        Some("Please fill out this field.")
    );
    // Canceled: neither.
    run(&mut tab, "document.activeElement.blur(); stop = true;");
    run(&mut tab, "document.getElementById('a').reportValidity();");
    assert_eq!(tab.engine.focused(), None);
    assert_eq!(tab.engine.take_notice(), None);
    // A form's: each control's event; reportValidity focuses the first.
    assert_eq!(
        run(
            &mut tab,
            "stop = false; console.log(document.getElementById('f').checkValidity());"
        ),
        ["invalid c true false", "invalid d true false", "false"]
    );
    run(&mut tab, "document.getElementById('f').reportValidity();");
    assert_eq!(tab.engine.focused(), Some(by_id(&tab.engine, "c")));
}

/// A server with `form` at `/form` and a page everywhere else.
fn site(form: &'static str) -> TestServer {
    serve(move |path| match path {
        "/form" => Some(html(form)),
        _ => Some(html("<p>done")),
    })
}

fn open(server: &TestServer, scripts: bool) -> Tab {
    let mut tab = Tab::new();
    tab.engine.set_scripting(scripts);
    load_all(&mut tab, server.url("/form"));
    tab.engine.frame(600, 300);
    lines(&mut tab);
    tab
}

#[test]
fn an_invalid_control_stops_a_submission() {
    let server = site(
        "<form id=f action=/sent><input id=q name=q required><input id=e name=e type=email>\
         <button id=go>Go</button><button id=skip formnovalidate>Skip</button></form>\
         <script>document.getElementById('q').addEventListener('invalid', \
           function () { console.log('invalid q'); });</script>",
    );
    let mut tab = open(&server, true);
    assert_eq!(
        run(&mut tab, "document.getElementById('go').click();"),
        ["invalid q"]
    );
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded, "not submitted");
    assert_eq!(tab.engine.focused(), Some(by_id(&tab.engine, "q")));
    assert_eq!(
        tab.engine.take_notice().as_deref(),
        Some("Please fill out this field.")
    );
    // The attempt set every control's user validity.
    assert_eq!(
        run(
            &mut tab,
            "console.log(document.querySelectorAll(':user-invalid').length)"
        ),
        ["1"]
    );
    // formnovalidate skips it; so does submit().
    run(&mut tab, "document.getElementById('skip').click();");
    assert_eq!(tab.engine.load_state(), &LoadState::Loading);
    let mut tab = open(&server, true);
    run(&mut tab, "document.getElementById('f').submit();");
    assert_eq!(tab.engine.load_state(), &LoadState::Loading);
    // A novalidate form submits.
    let server = site(
        "<form novalidate action=/sent><input name=q required><button id=go>Go</button></form>",
    );
    let mut tab = open(&server, true);
    run(&mut tab, "document.getElementById('go').click();");
    assert_eq!(tab.engine.load_state(), &LoadState::Loading);
}

#[test]
fn with_scripts_off_an_invalid_control_stops_a_submission() {
    let server = site("<form action=/sent><input id=q name=q required></form>");
    let mut tab = open(&server, false);
    tab.engine.key_down(&Key::new("Tab", "Tab"));
    tab.engine
        .key_down(&Key::new("Enter", "Enter").with_text("\r"));
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    assert_eq!(
        tab.engine.take_notice().as_deref(),
        Some("Please fill out this field.")
    );
}

#[test]
fn the_form_state_pseudo_classes_match() {
    let mut tab = page(
        "<form id=f><input id=req required><input id=fine value=x>\
         <input id=n type=number min=1 max=5 value=9><input id=r type=range>\
         <input id=ro readonly><input id=cb type=checkbox checked><button id=b>B</button>\
         <select id=s><option id=o selected>o</option></select></form><textarea id=t></textarea>\
         <form id=g><input id=gi></form>",
    );
    let matching = |tab: &mut Tab, selector: &str| {
        run(
            tab,
            &format!(
                "console.log(Array.prototype.map.call(document.querySelectorAll('{selector}'), \
                   function (e) {{ return e.id; }}).join(' '))"
            ),
        )
        .pop()
        .expect("the ids")
    };
    assert_eq!(matching(&mut tab, ":invalid"), "f req n");
    assert_eq!(matching(&mut tab, ":valid"), "fine r cb b s t g gi");
    assert_eq!(matching(&mut tab, ":in-range"), "r");
    assert_eq!(matching(&mut tab, ":out-of-range"), "n");
    assert_eq!(matching(&mut tab, ":read-write"), "req fine n t gi");
    assert_eq!(matching(&mut tab, "input:read-only"), "r ro cb");
    assert_eq!(matching(&mut tab, ":default"), "cb b o");
    assert_eq!(matching(&mut tab, ":user-invalid"), "");
    // The user's change committed: the field shows it.
    run(&mut tab, "document.getElementById('req').focus();");
    tab.engine.key_down(&Key::new("x", "KeyX").with_text("x"));
    tab.engine.key_down(&Key::new("Backspace", "Backspace"));
    tab.engine.key_down(&Key::new("y", "KeyY").with_text("y"));
    tab.engine.key_down(&Key::new("Tab", "Tab"));
    assert_eq!(matching(&mut tab, ":user-valid"), "req");
    run(&mut tab, "document.getElementById('req').value = '';");
    assert_eq!(matching(&mut tab, ":user-invalid"), "req");
    // Restyled as the value changes.
    let mut tab = page("<style>input:invalid { margin-left: 7px }</style><input id=i required>");
    assert_eq!(
        run(
            &mut tab,
            "var i = document.getElementById('i'); console.log(getComputedStyle(i).marginLeft); \
             i.value = 'x'; console.log(getComputedStyle(i).marginLeft);"
        ),
        ["7px", "0px"]
    );
}

#[test]
fn the_rest_of_the_states_and_messages() {
    let mut tab = page(
        "<input id=list type=email multiple value='a@b.c, d@e.f'>\
         <input id=one minlength=2 value=x><input id=edge type=number min=1 max=3 step=3 value=2>",
    );
    run(&mut tab, REPORT);
    run(&mut tab, "document.getElementById('one').focus();");
    tab.engine.key_down(&Key::new("End", "End"));
    tab.engine.key_down(&Key::new("Backspace", "Backspace"));
    tab.engine.key_down(&Key::new("y", "KeyY").with_text("y"));
    assert_eq!(
        run(&mut tab, "report('list'); report('one'); report('edge');"),
        [
            "list:  | true true | ",
            "one: tooShort | true false | Please lengthen this text to 2 characters or more (you are currently using 1 character).",
            "edge: stepMismatch | true false | Please enter a valid value. The nearest valid value is 1.",
        ]
    );
}

#[test]
fn more_of_the_pseudo_classes_and_their_restyles() {
    let mut tab = page(
        "<style>input:invalid, form:invalid, fieldset:invalid { margin-left: 7px }</style>\
         <form id=f><fieldset id=fs><input id=ok><input id=q required></fieldset><input id=z required>\
         <span><input type=radio name=g id=r1 required></span><input type=radio name=g id=r2>\
         <input id=free type=number><input id=off disabled>\
         <button id=b1>1</button><button id=b2>2</button></form>",
    );
    assert_eq!(
        run(
            &mut tab,
            "console.log(document.getElementById('fs').matches(':invalid'), \
               document.getElementById('free').matches(':in-range'), \
               document.getElementById('off').matches(':read-only'), \
               document.getElementById('b2').matches(':default'));"
        ),
        ["true false true false"]
    );
    // In one run, so that each read after a change is the journal's
    // restyle, not a whole cascade: the radio group restyled together
    // (r1 and r2 are not siblings), then the fieldset, though its form
    // stays invalid.
    assert_eq!(
        run(
            &mut tab,
            "function m(id) { return getComputedStyle(document.getElementById(id)).marginLeft; } \
             var before = [m('r2'), m('f'), m('fs')].join(); \
             document.getElementById('r1').checked = true; var group = m('r2'); \
             document.getElementById('q').value = 'x'; \
             console.log(before, group, m('f'), m('fs'));"
        ),
        ["7px,7px,7px 5px 7px 0px"]
    );
    assert_eq!(
        run(
            &mut tab,
            "console.log(document.getElementById('fs').matches(':valid'));"
        ),
        ["true"]
    );
    // A control outside its form, by `form`, restyles the form too.
    let mut tab = page(
        "<style>form:invalid { margin-left: 7px }</style>\
         <form id=f2></form><div><input id=w form=f2 required></div>",
    );
    assert_eq!(
        run(
            &mut tab,
            "function m(id) { return getComputedStyle(document.getElementById(id)).marginLeft; } \
             var before = m('f2'); document.getElementById('w').value = 'x'; \
             console.log(before, m('f2'));"
        ),
        ["7px 0px"]
    );
    // A checkbox clicked has its user validity.
    let mut tab = page("<input type=checkbox id=c required>");
    assert_eq!(
        run(
            &mut tab,
            "var c = document.getElementById('c'); c.click(); c.click(); \
             console.log(c.matches(':user-invalid'));"
        ),
        ["true"]
    );
}

#[test]
fn a_canceled_invalid_is_not_reported_and_novalidate_holds_with_scripts_off() {
    let mut tab = page(
        "<form id=f><input id=a required><input id=b required></form>\
         <script>document.getElementById('a').addEventListener('invalid', \
           function (e) { e.preventDefault(); });</script>",
    );
    run(&mut tab, "document.getElementById('f').reportValidity();");
    assert_eq!(
        tab.engine.focused(),
        Some(by_id(&tab.engine, "b")),
        "past the canceled one"
    );
    // Scripts off: novalidate submits; without it, the attempt marks the
    // controls.
    let server = site("<form novalidate action=/sent><input name=q required></form>");
    let mut tab = open(&server, false);
    tab.engine.key_down(&Key::new("Tab", "Tab"));
    tab.engine
        .key_down(&Key::new("Enter", "Enter").with_text("\r"));
    assert_eq!(tab.engine.load_state(), &LoadState::Loading);
    let server = site("<form action=/sent><input id=q name=q required></form>");
    let mut tab = open(&server, false);
    tab.engine.key_down(&Key::new("Tab", "Tab"));
    tab.engine
        .key_down(&Key::new("Enter", "Enter").with_text("\r"));
    let q = by_id(&tab.engine, "q");
    assert!(dom::validity::user_validity(tab.engine.document(), q));
}
