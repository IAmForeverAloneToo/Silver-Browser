//! The engine running a document's scripts (M24): the switch, inline
//! and external scripts in their order, the style sheets that hold them,
//! their responses, encodings and muted errors, the console, and a realm
//! out of memory.

use std::time::Duration;

use net::test_server::{Reply, TestServer};

use super::tests::{FIRST, load_all};
use crate::testing::*;
use crate::{Engine, LoadState};

/// `body` served and loaded with scripting `on`; the console's lines.
pub(super) fn scripted(body: &'static str, on: bool) -> (Tab, Vec<String>) {
    scripted_bytes(body.as_bytes().to_vec(), on.then(js::Limits::default))
}

/// `body` served and loaded, with scripts run with `limits` if any; the
/// console's lines.
pub(super) fn scripted_bytes(body: Vec<u8>, limits: Option<js::Limits>) -> (Tab, Vec<String>) {
    let server = serve(move |path| match path {
        "/page.html" => Some(Reply::ok("text/html", &body)),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(limits.is_some());
    if let Some(limits) = limits {
        tab.engine.script_limits = limits;
    }
    load_all(&mut tab, server.url("/page.html"));
    let console = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    // The server goes with the test; the page stays.
    drop(server);
    (tab, console)
}

#[test]
fn inline_scripts_run_with_scripting_on_and_not_otherwise() {
    // A script in the head, run before the document is shown, and one
    // after a script that let it be shown.
    let page = "<title>Before</title><script>console.log('hello')</script><p>x</p>\
                <script></script><script>document.title = 'After'</script>";
    let (tab, console) = scripted(page, true);
    assert_eq!(console, ["hello"]);
    assert_eq!(tab.engine.title(), "After");
    assert_eq!(
        tab.engine.scripts(),
        crate::ScriptState::Running { errors: 0 }
    );
    let (tab, console) = scripted(page, false);
    assert!(console.is_empty());
    assert_eq!(tab.engine.title(), "Before");
    assert_eq!(tab.engine.scripts(), crate::ScriptState::Off);
}

#[test]
fn only_classic_scripts_for_the_window_run() {
    let (_, console) = scripted(
        "<script type=module>console.log('module')</script>\
         <script type=text/plain>console.log('plain')</script>\
         <script nomodule>console.log('nomodule')</script>\
         <script src='data:text/javascript,console.log(%22src%22)'>console.log('text')</script>\
         <script for=x event=onload>console.log('for x')</script>\
         <script for=window event=onclick>console.log('onclick')</script>\
         <script for=' Window ' event='ONLOAD()'>console.log('window onload')</script>\
         <script event=onclick>console.log('event alone')</script>\
         <script language=javascript>console.log('language')</script>",
        true,
    );
    assert_eq!(console, ["src", "window onload", "event alone", "language"]);
}

#[test]
fn an_empty_script_makes_no_realm_and_one_that_cannot_be_made_stops_scripts() {
    // Too little heap for a realm: none is made for nothing to run.
    let small = js::Limits {
        heap: 64 << 10,
        ..js::Limits::default()
    };
    let (tab, console) = scripted_bytes(b"<script></script><p>x".to_vec(), Some(small));
    assert!(console.is_empty());
    assert_eq!(
        tab.engine.scripts(),
        crate::ScriptState::Running { errors: 0 }
    );
    let (mut tab, console) = scripted_bytes(b"<script>1</script><p>x".to_vec(), Some(small));
    assert!(matches!(
        tab.engine.scripts(),
        crate::ScriptState::Stopped { .. }
    ));
    assert_eq!(
        console.last().map(String::as_str),
        Some("Scripts stopped on this page: out of memory")
    );
    assert_eq!(
        tab.engine.take_notice().as_deref(),
        Some("Scripts stopped on this page: out of memory")
    );
}

#[test]
fn a_document_started_again_runs_its_scripts_in_a_new_realm() {
    let script = b"<script>console.log(typeof seen); var seen = 1</script>";
    // Before the body, while the document is the navigation's.
    let mut early = b"<head>".to_vec();
    early.extend_from_slice(script);
    early.extend_from_slice(b"<!--");
    early.extend(vec![b'.'; 1100]);
    early.extend_from_slice(b"--><meta charset=windows-1252></head><body><p>x");
    // After it, the document shown.
    let mut late = FIRST.as_bytes().to_vec();
    late.extend_from_slice(script);
    late.extend_from_slice(b"<meta charset=windows-1252><p>x");
    for page in [early, late] {
        let (tab, console) = scripted_bytes(page, Some(js::Limits::default()));
        assert_eq!(tab.engine.shown.page.encoding(), encoding::WINDOWS_1252);
        assert_eq!(console, ["undefined", "undefined"]);
    }
}

#[test]
fn a_script_sees_the_tree_as_it_stands_at_its_end_tag() {
    // As the reference browser has it: a title made by a script in the
    // head before the parsed one, which comes second and is not the title.
    let (tab, console) = scripted(
        "<script>console.log(JSON.stringify(document.title)); document.title = '  a \\n b  '</script>\
         <title>later</title><script>console.log(document.title)</script>",
        true,
    );
    assert_eq!(console, ["\"\"", "a b"]);
    assert_eq!(tab.engine.title(), "a b");
    let document = tab.engine.document();
    let titles = document
        .subtree(document.root())
        .filter(|&n| document.tag(n) == Some("title"))
        .count();
    assert_eq!(titles, 2);
}

#[test]
fn errors_are_reported_and_promise_jobs_run_after_each_script() {
    let (tab, console) = scripted(
        "<script>Promise.resolve().then(() => console.log('job')); console.log('script')</script>\n\
         <p>x</p>\n\
         <script>\nthrow new Error('boom')</script>\
         <script>console.log('after')</script>",
        true,
    );
    let url = tab.engine.url().unwrap().to_string();
    assert_eq!(
        console,
        [
            "script".to_owned(),
            "job".to_owned(),
            format!("Uncaught Error: boom at {url}:4:11"),
            "after".to_owned(),
        ]
    );
    assert_eq!(
        tab.engine.scripts(),
        crate::ScriptState::Running { errors: 1 }
    );
}

#[test]
fn noscript_and_the_scripting_feature_follow_the_switch() {
    let page = "<style>@media (scripting: enabled) { #t { color: #f00 } }</style>\
                <body><noscript><p>without scripts</p></noscript><p id=t>x</p>";
    let display = |engine: &mut Engine| {
        let noscript = element(engine, "noscript");
        engine.styles().get(noscript).expect("styled").display
    };
    let (mut tab, _) = scripted(page, true);
    assert_eq!(color(&mut tab.engine, "t"), RED);
    // The noscript's content is text, and the element draws nothing.
    let noscript = element(&tab.engine, "noscript");
    let document = tab.engine.document();
    let children: Vec<_> = document.children(noscript).collect();
    assert_eq!(children.len(), 1);
    assert!(document.is_text(children[0]));
    // Its computed `display` stays `inline`, as the reference browser's
    // does, and it makes no box (M25).
    assert_eq!(display(&mut tab.engine), style::computed::Display::Inline);
    assert!(
        !text(&tab.engine).is_empty()
            && !tab
                .engine
                .fragments(200.0)
                .iter()
                .any(|f| f.node == noscript),
        "no box"
    );
    let (mut tab, _) = scripted(page, false);
    let noscript = element(&tab.engine, "noscript");
    let document = tab.engine.document();
    let first = document.first_child(noscript).expect("a child");
    assert_eq!(document.tag(first), Some("p"));
    assert_eq!(color(&mut tab.engine, "t"), BLACK);
    assert_ne!(display(&mut tab.engine), style::computed::Display::None);
    // Switched on, the feature follows at once; the parse stays as it was.
    tab.engine.frame(200, 100);
    tab.engine.set_scripting(true);
    assert!(tab.engine.needs_frame());
    assert_eq!(color(&mut tab.engine, "t"), RED);
    tab.engine.frame(200, 100);
    tab.engine.set_scripting(true);
    assert!(!tab.engine.needs_frame());
}

#[test]
fn a_document_kept_keeps_its_realm_collected_and_one_whose_realm_died_is_not_kept() {
    let server = serve(|path| match path {
        "/a.html" => Some(html(
            "<script>console.log('a ran'); \
             { const g = []; for (let i = 0; i < 1000; i++) { const o = {}; o.o = o; g.push(o) } }\
             </script><p>A</p>",
        )),
        "/b.html" => Some(html("<p>B</p>")),
        "/dead.html" => Some(html(
            "<script>var kept = []; for (let i = 0; ; i++) kept.push('x'.repeat(1000) + i);</script>",
        )),
        _ => None,
    });
    let pages = |server: &TestServer| {
        let requests = server.requests().into_iter();
        requests.filter(|r| r.target == "/a.html").count()
    };
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.script_limits = js::Limits {
        heap: 8 << 20,
        ..js::Limits::default()
    };
    let a = server.url("/a.html");
    load_all(&mut tab, a.clone());
    // Its icon's fetch done too: a page left with a fetch in flight is
    // not kept.
    tab.settle();
    assert_eq!(tab.engine.take_console().len(), 1);
    let heap = tab.engine.shown.scripts.heap();
    load_all(&mut tab, server.url("/b.html"));
    tab.settle();
    tab.engine.go_back();
    tab.wait_until(|engine| engine.url() == Some(&a));
    // From the cache: its script not run again, its garbage collected.
    assert_eq!(pages(&server), 1);
    assert!(tab.engine.take_console().is_empty());
    assert!(tab.engine.shown.scripts.heap() < heap);
    // Its realm dead, a document is not kept.
    load_all(&mut tab, server.url("/dead.html"));
    assert!(matches!(
        tab.engine.scripts(),
        crate::ScriptState::Stopped { .. }
    ));
    assert_eq!(tab.engine.history.kept_len(), 1);
    load_all(&mut tab, server.url("/b.html"));
    tab.settle();
    assert_eq!(tab.engine.history.kept_len(), 1);
}

#[test]
fn a_realm_out_of_memory_stops_the_pages_scripts_with_a_notice() {
    let fill = "<script>var kept = []; for (let i = 0; ; i++) kept.push('x'.repeat(1000) + i);</script>\
                <script>console.log('never')</script>";
    let limits = js::Limits {
        heap: 8 << 20,
        ..js::Limits::default()
    };
    // In the head, before the document is shown, and in the body after
    // a script that let it be shown.
    for page in [
        format!("{fill}<p>still here</p>"),
        format!("<p>first</p><script></script>{fill}<p>still here</p>"),
    ] {
        let (mut tab, console) = scripted_bytes(page.into_bytes(), Some(limits));
        assert!(matches!(
            tab.engine.scripts(),
            crate::ScriptState::Stopped { .. }
        ));
        assert_eq!(
            tab.engine.take_notice().as_deref(),
            Some("Scripts stopped on this page: out of memory")
        );
        assert!(!console.iter().any(|line| line == "never"));
        assert!(text(&tab.engine).ends_with("still here"));
    }
}

/// A tab whose engine runs scripts.
fn scripting() -> Tab {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab
}

/// The texts of the console's entries since they were last taken.
fn lines(tab: &mut Tab) -> Vec<String> {
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

/// A `200 OK` of `content_type` with `body`, sent after `wait`.
fn later(content_type: &str, body: &[u8], wait: Duration) -> Reply {
    let mut bytes = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    bytes.extend_from_slice(body);
    Reply::Pieces(vec![(wait, bytes)])
}

fn javascript(body: &str) -> Reply {
    Reply::ok("text/javascript", body.as_bytes())
}

#[test]
fn an_external_script_holds_the_parser_until_it_has_run() {
    let server = serve(|path| match path {
        "/head.html" => Some(html(
            "<script src=a.js></script><title>t</title><script>console.log('inline')</script>",
        )),
        "/a.js" => Some(later(
            "text/javascript",
            b"console.log(JSON.stringify(document.title))",
            Duration::from_millis(300),
        )),
        "/body.html" => Some(html(
            "<p>x</p><script></script><script src=slow.js></script><title>t</title><p>after</p>",
        )),
        "/slow.js" => Some(later(
            "text/javascript",
            b"console.log(JSON.stringify(document.title))",
            Duration::from_millis(500),
        )),
        _ => None,
    });
    // In the head: the parser waits, so the script sees no title yet,
    // and the page is shown only once it has run.
    let head = server.url("/head.html");
    let mut tab = scripting();
    tab.engine.navigate(head.clone());
    tab.wait_until(|engine| engine.url() == Some(&head));
    assert_eq!(lines(&mut tab), ["\"\"", "inline"]);
    // In the body: the page is shown with what came before it, and
    // what follows waits, and so does the load.
    let body = server.url("/body.html");
    tab.engine.navigate(body.clone());
    tab.wait_until(|engine| engine.url() == Some(&body));
    assert!(lines(&mut tab).is_empty());
    assert_eq!(text(&tab.engine), "x");
    assert_eq!(tab.engine.load_state(), &LoadState::Loading);
    load_all(&mut tab, body);
    assert_eq!(lines(&mut tab), ["\"\""]);
    assert_eq!(text(&tab.engine), "xtafter");
}

#[test]
fn what_an_async_script_does_while_the_parser_waits_is_taken() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<p>x</p><script></script><script async src=quick.js></script>\
             <script src=slow.js></script>",
        )),
        "/quick.js" => Some(later(
            "text/javascript",
            b"document.title = 'quick'",
            Duration::from_millis(300),
        )),
        "/slow.js" => Some(later(
            "text/javascript",
            b"console.log('slow')",
            Duration::from_secs(20),
        )),
        _ => None,
    });
    let page = server.url("/page.html");
    let mut tab = scripting();
    tab.engine.navigate(page.clone());
    tab.wait_until(|engine| engine.url() == Some(&page));
    tab.wait_until(|engine| engine.title() == "quick");
    // Taken as any change is, for the next frame, the parser waiting.
    assert!(tab.engine.shown.document.take_changes().is_empty());
    tab.engine.stop();
}

#[test]
fn a_deferred_script_waits_for_the_end_of_a_shown_documents_parse() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<p>x</p><script></script><script defer src=d.js></script>\
             <script src=slow.js></script><script>console.log('inline')</script>",
        )),
        "/d.js" => Some(javascript("console.log('d')")),
        "/slow.js" => Some(later(
            "text/javascript",
            b"console.log('slow')",
            Duration::from_millis(400),
        )),
        _ => None,
    });
    let mut tab = scripting();
    load_all(&mut tab, server.url("/page.html"));
    assert_eq!(lines(&mut tab), ["slow", "inline", "d"]);
}

#[test]
fn deferred_scripts_run_in_order_once_parsed_and_async_ones_as_they_come() {
    let script = |name: &str, wait: u64| {
        later(
            "text/javascript",
            format!("console.log('{name}')").as_bytes(),
            Duration::from_millis(wait),
        )
    };
    let server = serve(move |path| match path {
        "/page.html" => Some(html(
            "<script defer src=d0.js></script>\
             <script defer src=d1.js></script><script defer src=d2.js></script>\
             <script async defer src=soon.js></script>\
             <script defer>console.log('inline defer')</script>\
             <script src=block.js></script><script>console.log('inline')</script><p>x</p>",
        )),
        "/d0.js" => Some(script("d0", 0)),
        "/d1.js" => Some(script("d1", 900)),
        "/d2.js" => Some(script("d2", 0)),
        "/soon.js" => Some(script("soon", 0)),
        "/block.js" => Some(script("block", 450)),
        _ => None,
    });
    let mut tab = scripting();
    // Loaded once all of them have run, the slow deferred one too.
    load_all(&mut tab, server.url("/page.html"));
    assert_eq!(
        lines(&mut tab),
        ["inline defer", "soon", "block", "inline", "d0", "d1", "d2"]
    );
}

#[test]
fn a_style_sheet_on_its_way_holds_the_scripts_after_it() {
    for (sheet, held) in [
        ("<link rel=stylesheet href=slow.css>", true),
        ("<style>@import url(slow.css);</style>", true),
        ("<link rel=stylesheet media=print href=slow.css>", false),
    ] {
        // The async script runs as it comes; the inline one waits for the
        // sheet, unless the sheet is for another medium. The sheet comes
        // well after the script: a test thread kept waiting past both
        // would take them in one poll, the sheet first.
        let page = format!(
            "{sheet}<script async src=soon.js></script><script>console.log('inline')</script>"
        );
        let server = serve(move |path| match path {
            "/page.html" => Some(html(&page)),
            "/slow.css" => Some(later("text/css", b"p {}", Duration::from_millis(1500))),
            "/soon.js" => Some(later(
                "text/javascript",
                b"console.log('soon')",
                Duration::from_millis(250),
            )),
            _ => None,
        });
        let mut tab = scripting();
        load_all(&mut tab, server.url("/page.html"));
        let expected = if held {
            ["soon", "inline"]
        } else {
            ["inline", "soon"]
        };
        assert_eq!(lines(&mut tab), expected, "{sheet}");
    }
}

#[test]
fn a_script_runs_only_from_an_ok_response_of_a_type_a_script_may_have() {
    let server = serve(|path| {
        let named = |fields: &[(&str, &str)], name: &str| {
            Reply::with_fields(200, fields, format!("console.log('{name}')").as_bytes())
        };
        let nosniff = ("X-Content-Type-Options", "nosniff");
        match path {
            "/page.html" => Some(html(
                "<script src=missing.js></script><script src=error.js></script>\
                 <script src=png.js></script><script src=csv.js></script>\
                 <script src=plain.js></script><script src=plain-nosniff.js></script>\
                 <script src=none.js></script><script src=none-nosniff.js></script>\
                 <script src=js-nosniff.js></script>\
                 <script src=''></script><script src='http://[x'></script>",
            )),
            "/error.js" => Some(Reply::with_fields(
                500,
                &[("Content-Type", "text/javascript")],
                b"console.log('error')",
            )),
            "/png.js" => Some(named(&[("Content-Type", "image/png")], "png")),
            "/csv.js" => Some(named(&[("Content-Type", "text/csv")], "csv")),
            "/plain.js" => Some(named(&[("Content-Type", "text/plain")], "plain")),
            "/plain-nosniff.js" => Some(named(
                &[("Content-Type", "text/plain"), nosniff],
                "plain nosniff",
            )),
            "/none.js" => Some(named(&[], "none")),
            "/none-nosniff.js" => Some(named(&[nosniff], "none nosniff")),
            "/js-nosniff.js" => Some(named(
                &[("Content-Type", "text/javascript"), nosniff],
                "js nosniff",
            )),
            _ => None,
        }
    });
    let mut tab = scripting();
    load_all(&mut tab, server.url("/page.html"));
    let failed = |name: &str, why: &str| {
        format!(
            "Failed to load script {}: {why}",
            server.url(&format!("/{name}"))
        )
    };
    assert_eq!(
        lines(&mut tab),
        [
            failed("missing.js", "status 404"),
            failed("error.js", "status 500"),
            failed("png.js", "image/png is not a script's type"),
            failed("csv.js", "text/csv is not a script's type"),
            "plain".to_owned(),
            failed("plain-nosniff.js", "text/plain is not a script's type"),
            "none".to_owned(),
            failed("none-nosniff.js", "no type, under nosniff"),
            "js nosniff".to_owned(),
        ]
    );
    // A script that could not be loaded is not a script's error.
    assert_eq!(
        tab.engine.scripts(),
        crate::ScriptState::Running { errors: 0 }
    );
}

#[test]
fn a_script_is_decoded_by_its_bom_its_type_its_charset_or_the_document() {
    let latin = b"console.log('\xE9')";
    let utf8 = b"console.log('\xC3\xA9')";
    let bom = b"\xEF\xBB\xBFconsole.log('\xC3\xA9')";
    let server = serve(move |path| match path {
        "/page.html" => Some(html(
            "<meta charset=windows-1252>\
             <script src=type.js></script>\
             <script src=attribute.js charset=utf-8></script>\
             <script src=document.js></script>\
             <script src=bom.js></script>\
             <script src=over.js charset=utf-8></script>",
        )),
        // Each says e acute only if decoded as it should be: the type over
        // the document, the attribute over the document, the document
        // last, a BOM over the type, the type over the attribute.
        "/type.js" => Some(Reply::ok("text/javascript; charset=utf-8", utf8)),
        "/attribute.js" => Some(Reply::ok("text/javascript", utf8)),
        "/document.js" => Some(Reply::ok("text/javascript", latin)),
        "/bom.js" => Some(Reply::ok("text/javascript; charset=windows-1252", bom)),
        "/over.js" => Some(Reply::ok("text/javascript; charset=windows-1252", latin)),
        _ => None,
    });
    let mut tab = scripting();
    load_all(&mut tab, server.url("/page.html"));
    assert_eq!(lines(&mut tab), ["\u{e9}"; 5]);
}

#[test]
fn errors_of_a_script_from_another_origin_are_muted() {
    let other = serve(|path| match path {
        "/throw.js" => Some(javascript("throw new Error('secret')")),
        _ => None,
    });
    let elsewhere = other.url("/throw.js").to_string();
    let page = format!(
        "<script src={elsewhere}></script><script src=redirect.js></script>\
         <script src=throw.js></script>\
         <script src=\"data:text/javascript,throw new Error('data')\"></script>"
    );
    let server = serve(move |path| match path {
        "/page.html" => Some(html(&page)),
        "/throw.js" => Some(javascript("throw new Error('here')")),
        "/redirect.js" => Some(Reply::with_fields(
            302,
            &[("Location", elsewhere.as_str())],
            b"",
        )),
        _ => None,
    });
    let mut tab = scripting();
    load_all(&mut tab, server.url("/page.html"));
    let lines = lines(&mut tab);
    assert_eq!(
        lines[..2],
        ["Uncaught Script error.", "Uncaught Script error."]
    );
    assert_eq!(
        lines[2],
        format!("Uncaught Error: here at {}:1:11", server.url("/throw.js"))
    );
    assert!(
        lines[3].starts_with("Uncaught Error: data at data:text/javascript,")
            && lines[3].ends_with(":1:11"),
        "{}",
        lines[3]
    );
    assert_eq!(lines.len(), 4);
}

#[test]
fn stopping_drops_the_scripts_on_their_way_and_runs_none_after() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<p>x</p><script></script><script src=slow.js></script>\
             <p>after</p><script>console.log('after')</script>",
        )),
        "/slow.js" => Some(later(
            "text/javascript",
            b"console.log('slow')",
            Duration::from_secs(20),
        )),
        _ => None,
    });
    let page = server.url("/page.html");
    let mut tab = scripting();
    tab.engine.navigate(page.clone());
    tab.wait_until(|engine| engine.url() == Some(&page) && !engine.shown.scripts.settled());
    tab.engine.stop();
    tab.wait_until(|engine| engine.load_state() == &LoadState::Loaded);
    assert!(lines(&mut tab).is_empty());
    assert!(text(&tab.engine).starts_with("xafter"));
}

#[test]
fn a_document_whose_realm_died_fetches_no_more_scripts() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<script>var kept = []; for (let i = 0; ; i++) kept.push('x'.repeat(1000) + i);</script>\
             <script src=a.js></script><p>x</p>",
        )),
        "/a.js" => Some(javascript("console.log('a')")),
        _ => None,
    });
    let mut tab = scripting();
    tab.engine.script_limits = js::Limits {
        heap: 8 << 20,
        ..js::Limits::default()
    };
    load_all(&mut tab, server.url("/page.html"));
    assert!(matches!(
        tab.engine.scripts(),
        crate::ScriptState::Stopped { .. }
    ));
    assert!(!paths(&server).iter().any(|path| path == "/a.js"));
}

/// A page `body`, under these `Content-Security-Policy` fields.
fn under(body: &str, policies: &[&str]) -> Reply {
    let mut fields = vec![("Content-Type", "text/html")];
    fields.extend(
        policies
            .iter()
            .map(|policy| ("Content-Security-Policy", *policy)),
    );
    Reply::with_fields(200, &fields, body.as_bytes())
}

#[test]
fn a_pages_policy_decides_which_of_its_scripts_run() {
    let other = serve(|path| match path {
        "/a.js" => Some(javascript("console.log('other')")),
        _ => None,
    });
    let elsewhere = other.url("/a.js").to_string();
    let page = format!(
        "<script>console.log('inline')</script><script src=self.js></script>\
         <script src={elsewhere}></script><script src=redirect.js></script>\
         <script src=\"data:text/javascript,console.log('data')\"></script>"
    );
    let away = elsewhere.clone();
    let server = serve(move |path| match path {
        "/page.html" => Some(under(&page, &["script-src 'self'"])),
        "/self.js" => Some(javascript("console.log('self')")),
        "/redirect.js" => Some(Reply::with_fields(302, &[("Location", away.as_str())], b"")),
        _ => None,
    });
    let mut tab = scripting();
    load_all(&mut tab, server.url("/page.html"));
    // The refusal says it once (M33.1), naming the first URL after a
    // redirect to another origin.
    let blocked =
        |url: &str| format!("Content-Security-Policy refused the script {url}: script-src 'self'");
    assert_eq!(
        lines(&mut tab),
        [
            "Content-Security-Policy refused an inline script: script-src 'self'".to_owned(),
            "self".to_owned(),
            blocked(&elsewhere),
            blocked(server.url("/redirect.js").as_str()),
            blocked("data:text/javascript,console.log('data')"),
        ]
    );
    assert!(
        other.requests().is_empty(),
        "nothing asked of another origin"
    );
}

#[test]
fn unsafe_inline_lets_inline_scripts_run_unless_a_nonce_takes_its_place() {
    for (policy, runs) in [
        ("script-src 'unsafe-inline'", true),
        ("img-src 'none'", true),
        // A nonce takes 'unsafe-inline' away (M33.1: and its own runs).
        ("script-src 'unsafe-inline' 'nonce-xyz'", false),
        ("script-src 'nonce-abc'", true),
    ] {
        let server = serve(move |path| match path {
            "/page.html" => Some(under(
                "<script nonce=abc>console.log('inline')</script>",
                &[policy],
            )),
            _ => None,
        });
        let mut tab = scripting();
        load_all(&mut tab, server.url("/page.html"));
        let expected = match runs {
            true => "inline".to_owned(),
            false => format!("Content-Security-Policy refused an inline script: {policy}"),
        };
        // The icon `img-src 'none'` refuses is not this test's (M33.1).
        let lines: Vec<String> = lines(&mut tab)
            .into_iter()
            .filter(|line| !line.contains("favicon.ico"))
            .collect();
        assert_eq!(lines, [expected], "{policy}");
    }
}

#[test]
fn a_meta_policy_in_the_head_holds_for_the_scripts_after_it() {
    let server = serve(|path| match path {
        "/head.html" => Some(html(
            "<head><script>console.log('before')</script>\
             <meta http-equiv=Content-Security-Policy content=\"script-src 'none'\">\
             <script>console.log('after')</script><script src=a.js></script></head>",
        )),
        "/body.html" => Some(html(
            "<body><meta http-equiv=content-security-policy content=\"script-src 'none'\">\
             <script>console.log('runs')</script>",
        )),
        "/report.html" => Some(Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/html"),
                ("Content-Security-Policy-Report-Only", "script-src 'none'"),
            ],
            b"<script>console.log('runs')</script>",
        )),
        "/a.js" => Some(javascript("console.log('a')")),
        _ => None,
    });
    let mut tab = scripting();
    load_all(&mut tab, server.url("/head.html"));
    assert_eq!(
        lines(&mut tab),
        [
            "before".to_owned(),
            "Content-Security-Policy refused an inline script: script-src 'none'".to_owned(),
            format!(
                "Content-Security-Policy refused the script {}: script-src 'none'",
                server.url("/a.js")
            ),
        ]
    );
    // A meta's policy is taken in once, however many scripts follow it.
    let many = format!(
        "<meta http-equiv=Content-Security-Policy content=\"script-src 'unsafe-inline'\">{}",
        "<script>console.log(1)</script>".repeat(net::csp::MAX_POLICIES + 2)
    );
    let crowd = serve(move |path| match path {
        "/many.html" => Some(html(&many)),
        _ => None,
    });
    load_all(&mut tab, crowd.url("/many.html"));
    assert_eq!(lines(&mut tab), ["1"; net::csp::MAX_POLICIES + 2]);
    // Outside the head, a meta says nothing; a policy that only reports
    // blocks nothing, and reports (M33.1).
    load_all(&mut tab, server.url("/body.html"));
    assert_eq!(lines(&mut tab), ["runs"]);
    load_all(&mut tab, server.url("/report.html"));
    assert_eq!(
        lines(&mut tab),
        [
            "Content-Security-Policy (report-only) would refuse an inline script: \
             script-src 'none'",
            "runs"
        ]
    );
}

// ---- Scripts that scripts insert, and document.write (M25) -----------------

#[test]
fn a_script_a_script_makes_runs_when_connected_and_only_once() {
    let (_, console) = scripted(
        "<p>x</p><script>\
         var s = document.createElement('script'); \
         s.textContent = 'window.x3 = (window.x3 || 0) + 1'; \
         console.log('before', window.x3); document.body.appendChild(s); \
         console.log('after', window.x3); \
         document.body.appendChild(s); document.body.removeChild(s); document.body.appendChild(s); \
         var empty = document.createElement('script'); document.body.appendChild(empty); \
         empty.appendChild(document.createTextNode('console.log(\"late\")')); \
         var blank = document.createElement('script'); document.body.appendChild(blank); \
         blank.text = 'console.log(\"by text\")'; console.log('text set'); \
         var copy = s.cloneNode(true); document.body.appendChild(copy); \
         var fresh = document.createElement('script'); fresh.text = 'console.log(\"fresh\")'; \
         var twin = fresh.cloneNode(true); document.body.appendChild(twin); \
         var d = document.createElement('div'); \
         d.innerHTML = '<script>console.log(\"fragment\")<\\/script>'; \
         document.body.appendChild(d.firstChild); \
         d.innerHTML = '<script>console.log(\"fragment copy\")<\\/script>'; \
         document.body.appendChild(d.firstChild.cloneNode(true)); \
         var holder = document.createElement('div'); \
         var b = document.createElement('script'); b.text = 'console.log(\"second\")'; \
         var a = document.createElement('script'); a.text = 'console.log(\"first\")'; \
         holder.append(a, b); \
         console.log('detached'); document.body.appendChild(holder); \
         console.log('once', window.x3);\
         </script>",
        true,
    );
    assert_eq!(
        console,
        [
            "before undefined",
            "after 1",
            "late",
            "by text",
            "text set",
            "fresh",
            "detached",
            "first",
            "second",
            "once 1"
        ]
    );
}

#[test]
fn current_script_and_ready_state_say_what_runs_and_where_the_load_stands() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<script id=head>console.log(document.readyState, document.currentScript.id); \
             var s = document.createElement('script'); s.id = 'made'; \
             s.text = 'console.log(document.currentScript.id)'; document.head.appendChild(s); \
             console.log(document.currentScript.id)</script>\
             <script defer src=late.js id=deferred></script><p>x</p>",
        )),
        "/late.js" => Some(javascript(
            "console.log(document.readyState, document.currentScript.id)",
        )),
        _ => None,
    });
    let mut tab = scripting();
    load_all(&mut tab, server.url("/page.html"));
    assert_eq!(
        lines(&mut tab),
        ["loading head", "made", "head", "interactive deferred"]
    );
}

#[test]
fn written_markup_goes_in_where_the_script_is_and_written_scripts_nest() {
    // The reference browser's order (scratchpad probe2): a written inline
    // script runs inside the write, its own writes before the rest of
    // what wrote it; a written external one holds what is written after
    // it until it has run; an async script's write is ignored.
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<script>\
             console.log('A: start');\
             document.write('<p id=p1>1</p><script>console.log(\"B: nested inline; p1 exists: \" + \
             !!document.getElementById(\"p1\")); document.write(\"<span id=inner>i</span>\");<\\/script><p id=p2>2</p>');\
             console.log('C: after first write; p2 exists: ' + !!document.getElementById('p2') + \
             '; inner exists: ' + !!document.getElementById('inner'));\
             document.write('<script src=\"ext.js\"><\\/script><p id=p3>3</p>');\
             console.log('D: after ext write; p3 exists: ' + !!document.getElementById('p3'));\
             document.write('<p id=p4>4</p>');\
             console.log('E: p4 exists: ' + !!document.getElementById('p4'));\
             </script><script>\
             console.log('F: next script; p3 ' + !!document.getElementById('p3') + ' p4 ' + \
             !!document.getElementById('p4') + ' fromext ' + !!document.getElementById('fromext'));\
             var order = []; for (var n = document.body.firstChild; n; n = n.nextSibling) \
             if (n.id) order.push(n.id); console.log('G: order ' + order.join(','));\
             </script><script async src=async.js></script>",
        )),
        "/ext.js" => Some(javascript(
            "console.log('ext ran; p2 exists: ' + !!document.getElementById('p2'));\
             document.write('<i id=fromext>e</i>');",
        )),
        "/async.js" => Some(javascript(
            "console.log('async write attempt'); document.write('<b id=asyncw>A</b>');",
        )),
        _ => None,
    });
    let mut tab = scripting();
    load_all(&mut tab, server.url("/page.html"));
    assert_eq!(
        lines(&mut tab),
        [
            "A: start",
            "B: nested inline; p1 exists: true",
            "C: after first write; p2 exists: true; inner exists: true",
            "D: after ext write; p3 exists: false",
            "E: p4 exists: false",
            "ext ran; p2 exists: true",
            "F: next script; p3 true p4 true fromext true",
            "G: order p1,inner,p2,fromext,p3,p4",
            "async write attempt",
        ]
    );
    let document = tab.engine.document();
    assert!(
        !document
            .subtree(document.root())
            .any(|n| document.attr(n, "id") == Some("asyncw"))
    );
    document.validate().unwrap();
}

#[test]
fn a_nested_scripts_error_is_the_pages_and_nesting_is_bounded() {
    let (_, console) = scripted(
        "<p>x</p><script>document.write('<script>throw new Error(\"w\")<\\/script>'); \
         console.log('after')</script>",
        true,
    );
    assert_eq!(console.len(), 2, "{console:?}");
    assert!(
        console[0].starts_with("Uncaught Error: w at "),
        "{console:?}"
    );
    assert_eq!(console[1], "after");
    // A nested script past the budget stops the run it is nested in, the
    // script after it running with a budget of its own.
    let limits = js::Limits {
        steps: 50,
        ..js::Limits::default()
    };
    let (_, console) = scripted_bytes(
        b"<p>x</p><script>try { document.write('<script>for (;;) {}<\\/script>'); } \
          catch (e) {} console.log('not reached')</script><script>console.log('next')</script>"
            .to_vec(),
        Some(limits),
    );
    assert_eq!(console.len(), 2, "{console:?}");
    assert!(console[0].starts_with("Script ran too long"), "{console:?}");
    assert_eq!(console[1], "next");
    // A script that writes a script that writes one, forty deep.
    let (_, console) = scripted(
        "<p>x</p><script>window.depth = 0; window.deeper = function () { depth++; \
         document.write('<script>deeper()<\\/script>'); }; deeper(); \
         console.log('depth', depth)</script>",
        true,
    );
    let most = crate::scripts::MAX_SCRIPT_NESTING;
    assert_eq!(
        console,
        [
            format!("A script nested more than {most} deep was not run"),
            format!("depth {most}"),
        ]
    );
}

#[test]
fn script_elements_text_and_async_read_and_write_as_html_says() {
    let (_, console) = scripted(
        "<script id=p>console.log(1)</script><script>\
         var p = document.getElementById('p'); var s = document.createElement('script'); \
         console.log(p.text, p.async, s.async); s.async = false; console.log(s.async); \
         s.async = true; console.log(s.async, s.getAttribute('async')); \
         s.async = false; console.log(s.async, s.hasAttribute('async')); \
         s.text = 'a'; s.appendChild(document.createElement('b')).textContent = 'b'; \
         console.log(s.text, s.textContent)</script>",
        true,
    );
    assert_eq!(
        console,
        [
            "1",
            "console.log(1) false true",
            "false",
            "true ",
            "false false",
            "a ab"
        ]
    );
}

#[test]
fn scripts_inserted_with_async_false_run_in_order_and_the_others_as_they_come() {
    let script = |name: &str, wait: u64| {
        later(
            "text/javascript",
            format!("console.log('{name}')").as_bytes(),
            Duration::from_millis(wait),
        )
    };
    let server = serve(move |path| match path {
        "/page.html" => Some(html(
            "<p>x</p><script>\
             ['slow', 'fast'].forEach(function (name) { var s = document.createElement('script'); \
             s.setAttribute('src', name + '-ordered.js'); s.async = false; \
             document.body.appendChild(s); });\
             ['slow', 'fast'].forEach(function (name) { var s = document.createElement('script'); \
             s.setAttribute('src', name + '-soon.js'); document.body.appendChild(s); });\
             console.log('inserted')</script>",
        )),
        "/slow-ordered.js" => Some(script("slow ordered", 600)),
        "/fast-ordered.js" => Some(script("fast ordered", 0)),
        "/slow-soon.js" => Some(script("slow soon", 300)),
        "/fast-soon.js" => Some(script("fast soon", 0)),
        _ => None,
    });
    let mut tab = scripting();
    load_all(&mut tab, server.url("/page.html"));
    let console = lines(&mut tab);
    assert_eq!(console[0], "inserted");
    let at = |name: &str| console.iter().position(|line| line == name).expect(name);
    assert!(at("slow ordered") < at("fast ordered"), "{console:?}");
    assert!(at("fast soon") < at("slow soon"), "{console:?}");
    assert_eq!(console.len(), 5, "{console:?}");
}

#[test]
fn a_policy_refuses_an_inserted_inline_script() {
    let server = serve(|path| match path {
        "/page.html" => Some(under(
            "<script src=insert.js></script>",
            &["script-src 'self'"],
        )),
        "/insert.js" => Some(javascript(
            "var s = document.createElement('script'); s.text = 'console.log(1)'; \
             document.head.appendChild(s); console.log('after');",
        )),
        _ => None,
    });
    let mut tab = scripting();
    load_all(&mut tab, server.url("/page.html"));
    assert_eq!(
        lines(&mut tab),
        [
            "Content-Security-Policy refused an inline script: script-src 'self'",
            "after"
        ]
    );
}

#[test]
fn a_sheet_just_before_a_script_holds_it_and_the_script_reads_it() {
    // The sheet and the script come in one slice of the parse: the sheet
    // is found before the script is prepared, so it holds the script,
    // which then reads its rules (M25).
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<link rel=stylesheet href=slow.css><style>p { margin-left: 3px }</style>\
             <script>console.log(getComputedStyle(document.documentElement).backgroundColor, \
             getComputedStyle(document.querySelector('style')).display)</script><p>x</p>",
        )),
        "/slow.css" => Some(later(
            "text/css",
            b"html { background-color: #f00 }",
            Duration::from_millis(300),
        )),
        _ => None,
    });
    let mut tab = scripting();
    load_all(&mut tab, server.url("/page.html"));
    assert_eq!(lines(&mut tab), ["rgb(255, 0, 0) none"]);
}

#[test]
fn device_pixel_ratio_is_the_displays_scale() {
    // M32.4: a script reads the display's scale, and the viewport in
    // CSS pixels.
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<script>console.log(devicePixelRatio, innerWidth); \
             requestAnimationFrame(() => console.log('frame', innerWidth))</script>",
        )),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.set_scale_factor(1.25);
    tab.engine.set_viewport(400, 300);
    load_all(&mut tab, server.url("/page.html"));
    let console: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(console, ["1.25 400"]);
    // A frame 800 device pixels wide is 640 CSS pixels to its callbacks.
    tab.engine
        .frame_at(800, 600, crate::FrameTime::at(Duration::from_secs(1)));
    let console: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(console, ["frame 640"]);
}
