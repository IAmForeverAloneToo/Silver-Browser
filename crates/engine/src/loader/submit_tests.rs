//! Form submission (M26.9, `docs/design/events.md`): the entry list in
//! a GET's query and a POST's body in each encoding, the submitter's
//! own attributes, the `submit` and `reset` events, `submit()`,
//! `requestSubmit()` and `reset()`, implicit submission, a POST's entry
//! and its resubmission page, scripts off, and the refusals.

use std::sync::Arc;

use net::test_server::{ServerRequest, TestServer};

use super::tests::load_all;
use crate::testing::*;
use crate::{Button, Key, LoadState};

/// The console's lines since they were last taken.
fn lines(tab: &mut Tab) -> Vec<String> {
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

/// A server with `form` at `/form`, and a page at every other path that
/// says which it is.
fn site(form: &'static str) -> TestServer {
    serve(move |path| match path {
        "/form" => Some(html(form)),
        _ => Some(html("<title>done</title><p>done")),
    })
}

/// The form page loaded, scripts on or off.
fn open(server: &TestServer, scripts: bool) -> Tab {
    let mut tab = Tab::new();
    tab.engine.set_scripting(scripts);
    load_all(&mut tab, server.url("/form"));
    tab.engine.frame(600, 300);
    lines(&mut tab);
    tab
}

/// Waits for the page at `path` to be shown, loaded.
fn arrive(tab: &mut Tab, path: &str) {
    tab.wait_until(|engine| {
        engine.url().is_some_and(|url| url.path_str() == path)
            && engine.load_state() == &LoadState::Loaded
    });
}

/// The last request the server had for `path`.
fn request(server: &TestServer, path: &str) -> ServerRequest {
    server
        .requests()
        .into_iter()
        .rev()
        .find(|request| request.target.split('?').next() == Some(path))
        .unwrap_or_else(|| panic!("no request for {path}"))
}

const FORM: &str = "<form action='/search?old=1#frag'>\
    <input name=q value='a b&amp;c'>\
    <input type=checkbox name=c checked><input type=checkbox name=u>\
    <input name=d disabled value=x><input value=noname>\
    <select name=s multiple><option selected>1</option><option>2</option>\
    <option selected value=three>3</option><option selected disabled>4</option></select>\
    <datalist><input name=in-list value=no></datalist>\
    <textarea name=t>l1\nl2</textarea>\
    <input type=radio name=r value=a><input type=radio name=r value=b checked>\
    <input type=hidden name=_charset_>\
    <button name=b1 value=one>One</button><button id=b2 name=b2 value=two>Two</button>\
    <input type=file name=f>\
    </form><datalist><input name=no form=x></datalist>";

#[test]
fn a_get_puts_the_entry_list_in_the_actions_query() {
    let server = site(FORM);
    let mut tab = open(&server, true);
    tab.engine
        .run_classic("document.getElementById('b2').click();");
    arrive(&mut tab, "/search");
    let sent = request(&server, "/search");
    assert_eq!(sent.method, "GET");
    assert_eq!(
        sent.target,
        "/search?q=a+b%26c&c=on&s=1&s=three&t=l1%0D%0Al2&r=b&_charset_=UTF-8&b2=two&f="
    );
    assert_eq!(
        tab.engine.url().and_then(|url| url.fragment()),
        Some("frag"),
        "the fragment kept"
    );
}

#[test]
fn a_post_sends_its_body_in_each_encoding() {
    for (enctype, content_type, body) in [
        (
            "",
            "application/x-www-form-urlencoded",
            "q=a+b&t=x%0D%0Ay".to_owned(),
        ),
        (
            "text/plain",
            "text/plain",
            "q=a b\r\nt=x\r\ny\r\n".to_owned(),
        ),
    ] {
        let form = Box::leak(
            format!(
                "<form method=POST enctype='{enctype}' action=/post>\
                 <input name=q value='a b'><textarea name=t>x\ny</textarea>\
                 <button id=go>Go</button></form>"
            )
            .into_boxed_str(),
        );
        let server = site(form);
        let mut tab = open(&server, true);
        tab.engine
            .run_classic("document.getElementById('go').click();");
        arrive(&mut tab, "/post");
        let sent = request(&server, "/post");
        assert_eq!(sent.method, "POST", "{enctype}");
        assert_eq!(sent.header("Content-Type"), Some(content_type), "{enctype}");
        assert_eq!(String::from_utf8_lossy(&sent.body), body, "{enctype}");
        assert!(tab.engine.can_go_back());
    }
    // multipart/form-data: a boundary of the reference browser's form, a
    // part an entry.
    let server = site(
        "<form method=post enctype=multipart/form-data action=/post>\
         <input name=q value=v><input type=file name=f><button id=go>Go</button></form>",
    );
    let mut tab = open(&server, true);
    tab.engine
        .run_classic("document.getElementById('go').click();");
    arrive(&mut tab, "/post");
    let sent = request(&server, "/post");
    let content_type = sent.header("Content-Type").expect("a type").to_owned();
    let boundary = content_type
        .strip_prefix("multipart/form-data; boundary=")
        .expect("a boundary");
    assert!(boundary.starts_with("----SilverFormBoundary"));
    assert_eq!(
        String::from_utf8_lossy(&sent.body),
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"q\"\r\n\r\nv\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"f\"; filename=\"\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n\r\n--{boundary}--\r\n"
        )
    );
}

#[test]
fn the_submitters_attributes_win_and_the_encoding_is_the_forms() {
    let server = site(
        "<form action=/a method=post accept-charset='nope windows-1252'>\
         <input name=e value='\u{e9}\u{65e5}'><input type=hidden name=_charset_>\
         <input type=submit id=go formaction=/b formmethod=get name=s value=S></form>",
    );
    let mut tab = open(&server, true);
    tab.engine
        .run_classic("document.getElementById('go').click();");
    arrive(&mut tab, "/b");
    let sent = request(&server, "/b");
    assert_eq!(sent.method, "GET");
    assert_eq!(
        sent.target,
        "/b?e=%E9%26%2326085%3B&_charset_=windows-1252&s=S"
    );
}

/// A form with a logger of its submit and reset events.
const EVENTS: &str = "<form id=f action=/sent><input name=q value=v>\
    <button id=go>Go</button><button id=r type=reset>R</button></form>\
    <form id=other><button id=foreign>F</button></form><div id=d></div>\
    <script>var stop = false; var f = document.getElementById('f');\
      f.addEventListener('submit', function (e) { \
        console.log('submit ' + (e.submitter ? e.submitter.id : null) + ' ' + \
          e.constructor.name + ' ' + e.isTrusted); if (stop) e.preventDefault(); });\
      f.addEventListener('reset', function (e) { console.log('reset ' + e.isTrusted); \
        if (stop) e.preventDefault(); });</script>";

#[test]
fn submit_and_reset_events_come_first_and_can_cancel() {
    let server = site(EVENTS);
    let mut tab = open(&server, true);
    let run = |tab: &mut Tab, source: &str| {
        tab.engine.run_classic(source);
        lines(tab)
    };
    assert_eq!(
        run(
            &mut tab,
            "stop = true; document.getElementById('go').click();"
        ),
        ["submit go SubmitEvent true"]
    );
    // The value typed, reset by the button but not when canceled.
    run(&mut tab, "f.elements.q.value = 'typed';");
    assert_eq!(
        run(
            &mut tab,
            "document.getElementById('r').click(); console.log(f.elements.q.value);"
        ),
        ["reset true", "typed"]
    );
    assert_eq!(
        run(
            &mut tab,
            "stop = false; f.reset(); console.log(f.elements.q.value);"
        ),
        ["reset true", "v"]
    );
    // requestSubmit() checks its submitter.
    assert_eq!(
        run(
            &mut tab,
            "try { f.requestSubmit(document.getElementById('d')); } catch (e) { console.log(e.name); } \
             try { f.requestSubmit(document.getElementById('foreign')); } catch (e) { console.log(e.name); } \
             stop = true; f.requestSubmit(); f.requestSubmit(document.getElementById('go'));"
        ),
        [
            "TypeError",
            "NotFoundError",
            "submit null SubmitEvent true",
            "submit go SubmitEvent true",
        ]
    );
    assert!(
        server
            .requests()
            .iter()
            .all(|r| !r.target.starts_with("/sent"))
    );
    // submit() fires nothing, and goes.
    assert!(run(&mut tab, "f.submit();").is_empty());
    arrive(&mut tab, "/sent");
    assert_eq!(request(&server, "/sent").target, "/sent?q=v");
}

#[test]
fn enter_submits_implicitly() {
    let enter = Key::new("Enter", "Enter").with_text("\r");
    // One field and no button: submitted.
    let server = site("<form action=/one><input id=a name=a value=1></form>");
    let mut tab = open(&server, true);
    tab.engine
        .run_classic("document.getElementById('a').focus();");
    tab.engine.key_down(&enter);
    arrive(&mut tab, "/one");
    assert_eq!(request(&server, "/one").target, "/one?a=1");
    // Two fields and no button: nothing.
    let server = site("<form action=/two><input id=a name=a><input name=b></form>");
    let mut tab = open(&server, true);
    tab.engine
        .run_classic("document.getElementById('a').focus();");
    tab.engine.key_down(&enter);
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded, "no navigation");
    // A default button: clicked, and the submitter.
    let server = site(
        "<form action=/three><input id=a name=a><input name=b>\
         <button type=button>no</button><button name=go value=yes>Go</button></form>\
         <script>document.querySelector('[name=go]').addEventListener('click', \
           function () { console.log('clicked'); });</script>",
    );
    let mut tab = open(&server, true);
    tab.engine
        .run_classic("document.getElementById('a').focus();");
    tab.engine.key_down(&enter);
    assert_eq!(lines(&mut tab), ["clicked"]);
    arrive(&mut tab, "/three");
    assert_eq!(request(&server, "/three").target, "/three?a=&b=&go=yes");
}

#[test]
fn with_scripts_off_a_click_submits_and_resets() {
    let server = site(
        "<style>body { margin: 0 } button, input { display: block; height: 20px }</style>\
         <form action=/off><input name=q value=v><button type=reset>R</button>\
         <button>Go</button></form>",
    );
    let mut tab = open(&server, false);
    let q = by_id_or_name(&tab, "q");
    let _ = dom::forms::set_value(&mut tab.engine.shown.document, q, "typed");
    // The field is 26px tall, each button 20px.
    tab.engine.pointer_down(5.0, 35.0, Button::Primary, 1);
    tab.engine.pointer_up(5.0, 35.0, Button::Primary, 1);
    assert_eq!(dom::forms::value(tab.engine.document(), q), "v", "reset");
    tab.engine.pointer_down(5.0, 55.0, Button::Primary, 1);
    tab.engine.pointer_up(5.0, 55.0, Button::Primary, 1);
    arrive(&mut tab, "/off");
    assert_eq!(request(&server, "/off").target, "/off?q=v");
}

/// The element named `name`.
fn by_id_or_name(tab: &Tab, name: &str) -> dom::NodeId {
    let document = tab.engine.document();
    document
        .subtree(document.root())
        .find(|&node| document.attr(node, "name") == Some(name))
        .expect("the element")
}

#[test]
fn a_posts_page_is_not_sent_again_by_a_reload_or_a_return() {
    let gate = Gate::new();
    let held = Arc::clone(&gate);
    let server = serve(move |path| match path {
        "/form" => Some(html(
            "<form method=post action=/posted><input name=q value=v>\
             <button id=go>Go</button></form>",
        )),
        // The posted page's picture never comes: leaving it aborts it,
        // so the back-forward cache does not keep it.
        "/posted" => Some(html(
            "<p>posted<img src=/held><a id=next href=/next>next</a>",
        )),
        "/held" => {
            held.wait();
            None
        }
        _ => Some(html("<p>next")),
    });
    let mut tab = open(&server, true);
    tab.engine
        .run_classic("document.getElementById('go').click();");
    tab.wait_until(|engine| engine.url().is_some_and(|url| url.path_str() == "/posted"));
    let posts = |server: &TestServer| {
        server
            .requests()
            .iter()
            .filter(|r| r.target == "/posted")
            .count()
    };
    assert_eq!(posts(&server), 1);
    tab.engine.reload();
    assert_eq!(tab.engine.title(), "Confirm Form Resubmission");
    assert_eq!(posts(&server), 1, "not sent again");
    // Away and back: the entry's page is the same notice.
    tab.engine.navigate(server.url("/next"));
    arrive(&mut tab, "/next");
    tab.engine.go_back();
    assert_eq!(tab.engine.title(), "Confirm Form Resubmission");
    assert_eq!(posts(&server), 1);
    gate.open();
}

#[test]
fn a_mail_action_is_refused_and_a_script_one_runs() {
    let server = site(
        "<form id=m action='mailto:a@example.com'><button id=go>Go</button></form>\
         <form id=j action='javascript:console.log(\"ran\")'><button id=js>J</button></form>",
    );
    let mut tab = open(&server, true);
    tab.engine
        .run_classic("document.getElementById('go').click();");
    assert!(tab.engine.take_notice().is_some());
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    tab.engine
        .run_classic("document.getElementById('js').click();");
    assert_eq!(lines(&mut tab), ["ran"]);
}

#[test]
fn an_implicit_submission_hears_submit_and_scripts_off_refuse_mail() {
    let enter = Key::new("Enter", "Enter").with_text("\r");
    let server = site(
        "<form id=f action=/implicit><input id=a name=a></form>\
         <script>document.getElementById('f').addEventListener('submit', \
           function (e) { console.log('submit ' + e.submitter); e.preventDefault(); });</script>",
    );
    let mut tab = open(&server, true);
    tab.engine
        .run_classic("document.getElementById('a').focus();");
    tab.engine.key_down(&enter);
    assert_eq!(lines(&mut tab), ["submit null"]);
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded, "no navigation");
    // Scripts off: a mail action is refused, the notice says so.
    let server = site(
        "<style>body { margin: 0 } button { display: block; height: 20px }</style>\
         <form action='mailto:a@example.com'><button>Go</button></form>",
    );
    let mut tab = open(&server, false);
    tab.engine.pointer_down(5.0, 5.0, Button::Primary, 1);
    tab.engine.pointer_up(5.0, 5.0, Button::Primary, 1);
    assert!(tab.engine.take_notice().is_some());
}

#[test]
fn a_form_its_document_submits_while_parsing_is_sent_with_its_body() {
    // The document is still on its way when its script submits: the
    // POST goes with its body, not as a GET (found by the adversarial
    // site, M33.8), and a `form-action` it breaks sends nothing.
    let server = serve(|path| match path {
        "/form" => Some(html(
            "<form id=f action=/sent method=post><input name=q value=1></form>\
             <script>document.getElementById('f').submit()</script>",
        )),
        "/guarded" => Some(html(
            "<meta http-equiv=Content-Security-Policy content=\"form-action 'none'\">\
             <form id=f action=/refused method=post></form>\
             <script>document.getElementById('f').submit(); console.log('stayed')</script>",
        )),
        _ => Some(html("<title>done</title><p>done")),
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/form"));
    arrive(&mut tab, "/sent");
    let sent = request(&server, "/sent");
    assert_eq!(
        (sent.method.as_str(), sent.body.as_slice()),
        ("POST", &b"q=1"[..])
    );
    tab.engine.navigate(server.url("/guarded"));
    arrive(&mut tab, "/guarded");
    tab.settle();
    assert!(
        server
            .requests()
            .iter()
            .all(|request| !request.target.starts_with("/refused"))
    );
}

#[test]
fn a_form_out_of_the_document_is_not_submitted() {
    let server = site("<form id=f action=/gone><input name=q></form>");
    let mut tab = open(&server, true);
    tab.engine.run_classic(
        "var f = document.getElementById('f'); f.remove(); f.submit(); f.requestSubmit();",
    );
    tab.engine.frame(600, 300);
    assert!(
        server
            .requests()
            .iter()
            .all(|r| !r.target.starts_with("/gone"))
    );
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
}

#[test]
fn an_empty_action_is_the_documents_url_not_the_base() {
    let server =
        site("<base href=/elsewhere/><form><input name=q value=v><button id=go>Go</button></form>");
    let mut tab = open(&server, true);
    tab.engine
        .run_classic("document.getElementById('go').click();");
    arrive(&mut tab, "/form");
    assert_eq!(request(&server, "/form").target, "/form?q=v");
}

#[test]
fn implicit_submission_stops_at_a_disabled_button_and_a_removed_form() {
    let enter = Key::new("Enter", "Enter").with_text("\r");
    let server = site(
        "<form action=/no><input id=a name=a><button id=b disabled>Go</button></form>\
         <script>document.getElementById('b').addEventListener('click', \
           function () { console.log('clicked'); });</script>",
    );
    let mut tab = open(&server, true);
    tab.engine
        .run_classic("document.getElementById('a').focus();");
    tab.engine.key_down(&enter);
    assert!(lines(&mut tab).is_empty(), "not even clicked");
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    // The submit event's listener takes the form out: it goes nowhere.
    let server = site(
        "<form id=f action=/no><input id=a name=a></form>\
         <script>document.getElementById('f').addEventListener('submit', \
           function () { this.remove(); });</script>",
    );
    let mut tab = open(&server, true);
    tab.engine
        .run_classic("document.getElementById('a').focus();");
    tab.engine.key_down(&enter);
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
}
