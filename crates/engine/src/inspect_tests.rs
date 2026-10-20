//! The DevTools' ends in an engine (M36): the panel's page and its
//! bridge, which no other page has, and a page answering the panel.

use devtools::{Json, Request};

use dom::NodeId;

use crate::testing::*;
use crate::{LoadState, Url};

/// The text of the shown document's element of id `id`.
fn text_of(tab: &Tab, id: &str) -> Option<String> {
    let document = &tab.engine.shown.document;
    document
        .subtree(document.root())
        .find(|&node| document.attr(node, "id") == Some(id))
        .map(|node| document.text_content(node))
}

#[test]
fn the_panel_asks_the_browser_and_shows_its_answer() {
    let mut tab = Tab::new();
    tab.engine.show_devtools_panel();
    assert!(tab.engine.is_devtools_panel());
    let mut messages = Vec::new();
    let deadline = std::time::Instant::now() + WAIT;
    while messages.is_empty() {
        tab.engine.poll_network();
        tab.engine.run_tasks();
        messages.extend(tab.engine.take_devtools());
        assert!(
            std::time::Instant::now() < deadline,
            "the panel sent nothing"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(*tab.engine.load_state(), LoadState::Loaded);
    assert_eq!(tab.engine.url().map(Url::as_str), Some(devtools::PANEL_URL));
    // The panel starts by asking which page it inspects.
    let deadline = std::time::Instant::now() + WAIT;
    let request = loop {
        let asked = messages
            .iter()
            .filter_map(|message| Request::parse(message).ok())
            .find(|request| request.method == "page.info");
        if let Some(request) = asked {
            break request;
        }
        tab.engine.run_tasks();
        messages.extend(tab.engine.take_devtools());
        assert!(std::time::Instant::now() < deadline, "{messages:?}");
    };
    assert_eq!(request.method, "page.info");
    let answer = devtools::result(
        request.id,
        Json::object([
            ("url", Json::from("https://a.test/")),
            ("title", Json::from("A <b>page</b>")),
        ]),
    );
    tab.engine.devtools_send(answer);
    let deadline = std::time::Instant::now() + WAIT;
    while text_of(&tab, "inspected").as_deref() != Some("A <b>page</b> - https://a.test/") {
        tab.engine.run_tasks();
        assert!(
            std::time::Instant::now() < deadline,
            "{:?}",
            text_of(&tab, "inspected")
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    // The page's text is the panel's text, never its markup.
    let document = &tab.engine.shown.document;
    assert!(
        document
            .subtree(document.root())
            .all(|node| document.tag(node) != Some("b"))
    );
}

#[test]
fn no_other_page_has_the_bridge() {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    let page =
        "data:text/html,<script>console.log(typeof devtools, typeof window.devtools)</script>";
    tab.engine.navigate(Url::parse(page).expect("a URL"));
    let lines = tab.run_until(|lines| !lines.is_empty());
    assert_eq!(lines, ["undefined undefined"]);
    assert!(!tab.engine.is_devtools_panel());
    tab.engine.devtools_send("{\"event\":\"x\"}".to_owned());
    assert!(tab.engine.take_devtools().is_empty());
    // Navigated to, the panel's URL is no page of the browser's: it has
    // no bridge either.
    tab.engine
        .navigate(Url::parse(devtools::PANEL_URL).expect("a URL"));
    tab.settle();
    assert!(!tab.engine.shown.page.is_panel());
    assert!(!tab.engine.is_devtools_panel());
}

#[test]
fn a_page_answers_the_panel() {
    let mut tab = Tab::new();
    tab.load(Url::parse("data:text/html,<title>Hello</title>").expect("a URL"));
    tab.engine
        .inspect(r#"{"id":4,"method":"page.info","params":{}}"#);
    tab.engine.inspect(r#"{"id":5,"method":"page.nothing"}"#);
    // What is not a request has no answer.
    tab.engine.inspect(r#"{"id":"6","method":"page.info"}"#);
    tab.engine.inspect("not json");
    let answers = tab.engine.take_inspected();
    assert_eq!(answers.len(), 2, "{answers:?}");
    let info = Json::parse(&answers[0]).expect("JSON");
    assert_eq!(info.get("id").and_then(Json::as_u64), Some(4));
    let result = info.get("result").expect("a result");
    assert_eq!(result.get("title").and_then(Json::as_str), Some("Hello"));
    assert_eq!(
        result.get("url").and_then(Json::as_str),
        Some("data:text/html,<title>Hello</title>")
    );
    assert_eq!(answers[1], r#"{"id":5,"error":"no method page.nothing"}"#);
    assert!(tab.engine.take_inspected().is_empty());
}

#[test]
fn the_panels_engine_navigated_elsewhere_gives_that_page_no_bridge() {
    let mut tab = Tab::new();
    tab.engine.show_devtools_panel();
    tab.settle();
    let page = "data:text/html,<script>console.log(typeof devtools)</script>";
    tab.engine.navigate(Url::parse(page).expect("a URL"));
    let lines = tab.run_until(|lines| lines.iter().any(|line| line == "undefined"));
    assert!(!lines.iter().any(|line| line == "object"), "{lines:?}");
    assert!(!tab.engine.shown.page.is_panel());
}

/// A server of `page` at every path but `/missing.png`, which is a 404.
fn serve(page: &'static str) -> net::test_server::TestServer {
    net::test_server::TestServer::plain(move |request| {
        if request.target == "/missing.png" {
            return net::test_server::Reply::with_fields(404, &[], b"");
        }
        net::test_server::Reply::ok("text/html", page.as_bytes())
    })
}

/// The entries of a page with scripts on until one's text is `last`.
fn entries_until(tab: &mut Tab, last: &str) -> Vec<crate::ConsoleEntry> {
    let deadline = std::time::Instant::now() + WAIT;
    let mut entries = Vec::new();
    loop {
        tab.engine.poll_network();
        tab.engine.run_tasks();
        entries.extend(tab.engine.take_console());
        if entries.iter().any(|entry| entry.text == last) {
            return entries;
        }
        assert!(std::time::Instant::now() < deadline, "{entries:#?}");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// The previews of `entry`, parsed.
fn args(entry: &crate::ConsoleEntry) -> Vec<Json> {
    match Json::parse(&entry.previews).expect("JSON previews") {
        Json::Array(items) => items,
        other => panic!("{other:?}"),
    }
}

fn field<'a>(value: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(value, |value, name| {
        value
            .get(name)
            .unwrap_or_else(|| panic!("no {name} in {value:?}"))
    })
}

/// The preview of property `name` of an object's preview.
fn property<'a>(object: &'a Json, name: &str) -> &'a Json {
    object
        .get("properties")
        .and_then(Json::as_array)
        .and_then(|props| {
            props.iter().find_map(|pair| match pair.as_array() {
                Some([key, value]) if key.as_str() == Some(name) => Some(value),
                _ => None,
            })
        })
        .unwrap_or_else(|| panic!("no {name} in {object:?}"))
}

const PREVIEWED: &str = "<body><div id=d class='a b'></div><script>
const o = { n: 1, s: 'x', nested: { deeper: { deepest: { gone: 1 } } }, list: [1, 2] };
o.self = o;
Object.defineProperty(o, 'g', { get() { console.log('a getter ran'); return 1; }, enumerable: true });
const d = document.getElementById('d'), e = new TypeError('bad'), m = new Map([[1, 'one']]);
console.log('%s and', 'text', 1.5, -0, NaN, 10n, Symbol('q'), null, undefined, true, o, d, e, m, new Set(['s']), function named() {}, new Date(0), /re/g);
console.log('done');
</script>";

#[test]
fn a_console_entry_keeps_its_arguments_previews_and_its_place() {
    let server = serve(PREVIEWED);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/"));
    let entries = entries_until(&mut tab, "done");
    assert!(
        !entries.iter().any(|entry| entry.text == "a getter ran"),
        "no getter runs"
    );
    let entry = &entries[0];
    assert_eq!(entry.source, crate::ConsoleSource::Console);
    // Its place is its call's line. (QuickJS gives a call over several
    // lines its last line, where the reference browser gives its first.)
    assert_eq!(
        (entry.url.as_str(), entry.line),
        (server.url("/").as_str(), 6)
    );
    let args = args(entry);
    let types: Vec<&str> = args
        .iter()
        .map(|arg| field(arg, &["type"]).as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        types,
        [
            "string",
            "string",
            "number",
            "number",
            "number",
            "bigint",
            "symbol",
            "null",
            "undefined",
            "boolean",
            "object",
            "node",
            "error",
            "map",
            "set",
            "function",
            "date",
            "regexp"
        ]
    );
    assert_eq!(field(&args[3], &["description"]).as_str(), Some("-0"));
    assert_eq!(field(&args[5], &["description"]).as_str(), Some("10n"));
    let object = &args[10];
    assert_eq!(
        property(object, "n").get("value").and_then(Json::as_f64),
        Some(1.0)
    );
    assert_eq!(
        property(object, "g").get("type").and_then(Json::as_str),
        Some("accessor")
    );
    assert_eq!(
        property(object, "self").get("circular"),
        Some(&Json::Bool(true))
    );
    let deeper = property(property(object, "nested"), "deeper");
    assert_eq!(
        property(deeper, "deepest").get("collapsed"),
        Some(&Json::Bool(true))
    );
    assert_eq!(
        field(property(object, "list"), &["length"]).as_u64(),
        Some(2)
    );
    let node = &args[11];
    assert_eq!(field(node, &["tag"]).as_str(), Some("div"));
    assert_eq!(field(node, &["id"]).as_str(), Some("d"));
    assert_eq!(field(node, &["classes"]).as_str(), Some("a b"));
    assert_eq!(field(&args[12], &["class"]).as_str(), Some("TypeError"));
    assert_eq!(field(&args[12], &["message"]).as_str(), Some("bad"));
    assert_eq!(field(&args[13], &["size"]).as_u64(), Some(1));
    assert_eq!(field(&args[15], &["name"]).as_str(), Some("named"));
    assert_eq!(
        field(&args[16], &["description"]).as_str(),
        Some("1970-01-01T00:00:00.000Z")
    );
}

const STANDARD: &str = "<script>
function inner() { console.trace('here'); }
function outer() { inner(); }
outer();
console.assert(1 === 2, 'not %s', 'equal');
console.assert(true, 'never');
console.count(); console.count(); console.count('x'); console.countReset();
console.count();
console.group('g'); console.log('in'); console.groupEnd(); console.groupEnd();
console.table([{ a: 1 }]);
console.dir({ d: 1 });
console.time('t'); console.timeEnd('t'); console.timeEnd('t');
console.clear();
function thrower() { throw new RangeError('deep'); }
setTimeout(() => { console.log('done'); }, 0);
thrower();
</script>";

#[test]
fn the_console_standards_methods_and_an_errors_stack() {
    use js::ConsoleKind::*;
    let server = serve(STANDARD);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/"));
    let entries = entries_until(&mut tab, "done");
    let texts: Vec<&str> = entries.iter().map(|entry| entry.text.as_str()).collect();
    let trace = &entries[0];
    assert_eq!((trace.kind, trace.text.as_str()), (Trace, "here"));
    let functions: Vec<&str> = trace.stack.iter().map(|f| f.function.as_str()).collect();
    assert_eq!(functions[..2], ["inner", "outer"], "{:?}", trace.stack);
    assert_eq!(
        (trace.stack[0].line, trace.url.as_str()),
        (2, server.url("/").as_str())
    );
    assert_eq!(texts[1], "Assertion failed: not equal");
    assert_eq!(entries[1].level, js::ConsoleLevel::Error);
    assert!(!entries[1].stack.is_empty());
    assert_eq!(texts[2..5], ["default: 1", "default: 2", "x: 1"]);
    assert_eq!(texts[5], "default: 1");
    assert_eq!((entries[6].kind, texts[6]), (Group, "g"));
    assert_eq!(texts[7], "in");
    assert_eq!(entries[8].kind, GroupEnd);
    // The second groupEnd closes nothing, and says nothing.
    assert_eq!(entries[9].kind, Table);
    assert_eq!(entries[10].kind, Dir);
    assert!(
        texts[11].starts_with("t: ") && texts[11].ends_with(" ms - timer ended"),
        "{}",
        texts[11]
    );
    assert_eq!(texts[12], "Timer 't' does not exist");
    assert_eq!(entries[13].kind, Clear);
    // The uncaught exception keeps its stack.
    let thrown = &entries[14];
    assert_eq!(thrown.source, crate::ConsoleSource::Exception);
    assert!(
        thrown.text.starts_with("Uncaught RangeError: deep"),
        "{}",
        thrown.text
    );
    assert_eq!(thrown.stack[0].function, "thrower");
    assert_eq!(thrown.stack[0].line, 14);
    assert_eq!(texts[15], "done");
}

const LIMITED: &str = "<script>
let prepared = 0;
Error.prepareStackTrace = (error, frames) => { prepared++; return 'prepared'; };
Error.stackTraceLimit = 0;
function deep(n) { if (n === 0) { console.trace('t'); return; } deep(n - 1); }
deep(20);
console.log('plain', { a: 1 }, new Date(0), new Map());
console.log('q\"\\\\\\n\\u2028\\ud800');
console.log('after', prepared, Error.stackTraceLimit, typeof Error.prepareStackTrace);
console.log('done');
</script>";

#[test]
fn a_console_call_keeps_its_place_whatever_the_page_did_to_errors_stacks() {
    let server = serve(LIMITED);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/"));
    let entries = entries_until(&mut tab, "done");
    // A trace has all its frames, past the page's limit and the
    // default's ten.
    let trace = &entries[0];
    let deep = trace.stack.iter().filter(|f| f.function == "deep").count();
    assert_eq!(deep, 21, "{:?}", trace.stack);
    assert_eq!((trace.line, trace.stack[0].line), (5, 5));
    // A call's place, its limit zero.
    let plain = &entries[1];
    assert_eq!(plain.line, 7, "{plain:?}");
    let previews = args(plain);
    let types: Vec<&str> = previews
        .iter()
        .map(|arg| field(arg, &["type"]).as_str().unwrap_or_default())
        .collect();
    assert_eq!(types, ["string", "object", "date", "map"]);
    // A string's preview, its escapes written back: a lone surrogate
    // read as U+FFFD.
    let quoted = args(&entries[2]);
    assert_eq!(
        field(&quoted[0], &["value"]).as_str(),
        Some("q\"\\\n\u{2028}\u{FFFD}")
    );
    // The page's prepareStackTrace never ran, and both are the page's
    // again after.
    assert_eq!(entries[3].text, "after 0 0 function");
}

#[test]
fn the_console_evaluates_in_the_page_its_policy_aside() {
    let server = net::test_server::TestServer::plain(|_| {
        net::test_server::Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/html"),
                ("Content-Security-Policy", "script-src 'unsafe-inline'"),
            ],
            b"<script>var answer = 41; console.log('ready')</script>",
        )
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/"));
    entries_until(&mut tab, "ready");
    let ask = |tab: &mut Tab, id: u64, expression: &str| {
        let request = devtools::Request {
            id,
            method: "console.evaluate".to_owned(),
            params: Json::object([("expression", Json::from(expression))]),
        };
        tab.engine.inspect(&request.write());
        let answer = tab.engine.take_inspected().pop().expect("an answer");
        Json::parse(&answer).expect("JSON")
    };
    let answer = ask(&mut tab, 1, "answer + 1");
    assert_eq!(
        field(&answer, &["result", "result", "value"]).as_f64(),
        Some(42.0),
        "{answer:?}"
    );
    let answer = ask(&mut tab, 2, "throw new TypeError('no')");
    assert_eq!(
        field(&answer, &["result", "exception", "message"]).as_str(),
        Some("no")
    );
    // `$0` is nothing while no node is selected, and is gone after.
    let answer = ask(&mut tab, 3, "typeof $0");
    assert_eq!(
        field(&answer, &["result", "result", "value"]).as_str(),
        Some("undefined")
    );
    // A declaration stays, as a script's does.
    ask(&mut tab, 5, "var kept = 7");
    let answer = ask(&mut tab, 6, "kept");
    assert_eq!(
        field(&answer, &["result", "result", "value"]).as_f64(),
        Some(7.0)
    );
    // With scripts off, nothing runs.
    let mut off = Tab::new();
    off.load(server.url("/"));
    off.engine
        .inspect(r#"{"id":1,"method":"console.evaluate","params":{"expression":"1"}}"#);
    assert_eq!(
        off.engine.take_inspected(),
        [r#"{"id":1,"error":"scripts do not run on this page"}"#]
    );
}

#[test]
fn a_resource_that_fails_is_said_on_the_console() {
    let server = serve("<img src=/missing.png>");
    let mut tab = Tab::new();
    tab.engine.navigate(server.url("/"));
    let deadline = std::time::Instant::now() + WAIT;
    let mut entries = Vec::new();
    while !entries
        .iter()
        .any(|entry: &crate::ConsoleEntry| entry.source == crate::ConsoleSource::Network)
    {
        assert!(std::time::Instant::now() < deadline, "{entries:#?}");
        tab.engine.poll_network();
        entries.extend(tab.engine.take_console());
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let failed = &entries[0];
    assert_eq!(
        failed.text,
        format!(
            "Failed to load resource: the server responded with a status of 404 ({})",
            server.url("/missing.png")
        )
    );
    assert_eq!(failed.url, server.url("/missing.png").as_str());
}

const BOUNDED: &str = "<script>
const many = {};
for (let i = 0; i < 150; i++) { many['p' + i] = i; }
console.log(many);
console.log('x'.repeat(10000), 'x'.repeat(10000), 'x'.repeat(10000), 'x'.repeat(10000), 'x'.repeat(10000), 'x'.repeat(10000), 'x'.repeat(10000));
console.log('done');
</script>";

#[test]
fn a_preview_is_bounded_and_a_log_keeps_its_place_alone() {
    let server = serve(BOUNDED);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/"));
    let entries = entries_until(&mut tab, "done");
    let many = &args(&entries[0])[0];
    assert_eq!(
        many.get("properties")
            .and_then(Json::as_array)
            .map(<[Json]>::len),
        Some(100)
    );
    assert_eq!(many.get("more"), Some(&Json::Bool(true)));
    // A log has its place but not its stack, which an error keeps.
    assert_eq!(entries[0].line, 4);
    assert!(entries[0].stack.is_empty());
    // Past the previews' bound, an entry has its text alone.
    assert!(entries[1].previews.is_empty());
    assert_eq!(entries[1].text.len(), crate::MAX_CONSOLE_MESSAGE);
}

#[test]
fn the_consoles_dollar_zero_is_gone_after_the_expression() {
    let server = serve("<script>console.log('ready')</script>");
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/"));
    entries_until(&mut tab, "ready");
    tab.engine.inspect(
        r#"{"id":1,"method":"console.evaluate","params":{"expression":"setTimeout(() => console.log('after ' + ('$0' in window)), 0)"}}"#,
    );
    let entries = entries_until(&mut tab, "after false");
    assert!(!entries.iter().any(|entry| entry.text == "after true"));
}

/// What the page answers `method` with `params`: its result, or why not.
fn ask(tab: &mut Tab, method: &str, params: Json) -> Result<Json, String> {
    let request = devtools::Request {
        id: 1,
        method: method.to_owned(),
        params,
    };
    tab.engine.inspect(&request.write());
    let answers = tab.engine.take_inspected();
    let answer = answers
        .iter()
        .rev()
        .find(|answer| answer.starts_with(r#"{"id":1,"#))
        .unwrap_or_else(|| panic!("no answer to {method}: {answers:?}"));
    let answer = Json::parse(answer).expect("JSON");
    match answer.get("result") {
        Some(result) => Ok(result.clone()),
        None => Err(answer
            .get("error")
            .and_then(Json::as_str)
            .unwrap_or_default()
            .to_owned()),
    }
}

/// The number of the first element of tag `tag`.
fn node_of(tab: &Tab, tag: &str) -> u64 {
    let document = &tab.engine.shown.document;
    document
        .subtree(document.root())
        .find(|&node| document.tag(node) == Some(tag))
        .unwrap_or_else(|| panic!("no {tag}"))
        .to_bits()
}

fn children(node: &Json) -> &[Json] {
    node.get("children")
        .and_then(Json::as_array)
        .unwrap_or_default()
}

fn name(node: &Json) -> &str {
    node.get("name").and_then(Json::as_str).unwrap_or_default()
}

const INSPECTED: &str = "<!doctype html><html><head><style>
p { color: red; margin: 0 }
.x { color: green }
</style></head>
<body>
  <p class=x id=p style='padding: 4px'>Text <b>bold</b></p>
  <div id=d onmousedown=\"document.title += 'd'\" onmouseup=\"document.title += 'u'\" onclick=\"document.title += 'c'\" style='width: 100px; height: 50px; background: #fff'></div>
</body></html>";

/// A tab showing `page`, laid out and drawn at 320 by 240.
fn inspected(page: &'static str) -> (Tab, net::test_server::TestServer) {
    let server = serve(page);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.load(server.url("/"));
    tab.engine.frame(320, 240);
    (tab, server)
}

#[test]
fn the_tree_is_given_as_deep_as_it_is_open() {
    let (mut tab, _server) = inspected(INSPECTED);
    let tree = ask(&mut tab, "dom.tree", Json::object([])).expect("a tree");
    assert_eq!(name(&tree), "#document");
    let html = &children(&tree)[0];
    assert_eq!(name(html), "html");
    let names: Vec<&str> = children(html).iter().map(name).collect();
    assert_eq!(names, ["head", "body"]);
    // The head is closed; the body open, its white space left out.
    assert!(children(html)[0].get("children").is_none());
    assert_eq!(
        children(html)[0].get("childCount").and_then(Json::as_u64),
        Some(1)
    );
    let body = &children(html)[1];
    let names: Vec<&str> = children(body).iter().map(name).collect();
    assert_eq!(names, ["p", "div"]);
    let p = &children(body)[0];
    assert_eq!(
        p.get("attributes"),
        Some(&Json::parse(r#"[["class","x"],["id","p"],["style","padding: 4px"]]"#).unwrap())
    );
    // A node asked for, opened where the panel opened it.
    let p_id = p.get("id").and_then(Json::as_u64).expect("an id");
    let p = ask(
        &mut tab,
        "dom.tree",
        Json::object([("node", Json::from(p_id)), ("open", Json::Array(vec![]))]),
    )
    .expect("a node");
    let kids = children(&p);
    assert_eq!(kids[0].get("text").and_then(Json::as_str), Some("Text "));
    assert_eq!(kids[0].get("type").and_then(Json::as_u64), Some(3));
    assert_eq!(name(&kids[1]), "b");
    assert!(
        ask(
            &mut tab,
            "dom.tree",
            Json::object([("node", Json::from(12345678u64))])
        )
        .is_err()
    );
}

#[test]
fn the_rules_that_matched_and_the_computed_values_are_given() {
    let (mut tab, server) = inspected(INSPECTED);
    let p = node_of(&tab, "p");
    let matched = ask(
        &mut tab,
        "css.matched",
        Json::object([("node", Json::from(p))]),
    )
    .expect("rules");
    let inline = matched.get("inline").expect("inline");
    assert_eq!(
        inline
            .get("declarations")
            .and_then(|d| d.as_array())
            .map(<[Json]>::len),
        Some(1)
    );
    let rules = matched
        .get("rules")
        .and_then(Json::as_array)
        .expect("rules");
    let selectors: Vec<&str> = rules
        .iter()
        .map(|rule| {
            rule.get("selector")
                .and_then(Json::as_str)
                .unwrap_or_default()
        })
        .collect();
    assert_eq!(selectors, [".x", "p", "p"]);
    assert_eq!(
        rules[0].get("source").and_then(Json::as_str),
        Some("<style>")
    );
    assert_eq!(
        rules[2].get("source").and_then(Json::as_str),
        Some("user agent stylesheet")
    );
    let overridden = |rule: &Json, name: &str| {
        rule.get("declarations")
            .and_then(Json::as_array)
            .and_then(|decls| {
                decls
                    .iter()
                    .find(|d| d.get("name").and_then(Json::as_str) == Some(name))
            })
            .and_then(|d| d.get("overridden"))
            .and_then(Json::as_bool)
    };
    assert_eq!(overridden(&rules[0], "color"), Some(false));
    assert_eq!(overridden(&rules[1], "color"), Some(true));
    let computed = ask(
        &mut tab,
        "css.computed",
        Json::object([("node", Json::from(p))]),
    )
    .expect("values");
    let value = |name: &str| {
        computed
            .as_array()
            .and_then(|pairs| {
                pairs.iter().find_map(|pair| match pair.as_array() {
                    Some([n, v]) if n.as_str() == Some(name) => v.as_str(),
                    _ => None,
                })
            })
            .map(str::to_owned)
    };
    assert_eq!(value("color").as_deref(), Some("rgb(0, 128, 0)"));
    assert_eq!(value("padding-top").as_deref(), Some("4px"));
    drop(server);
}

#[test]
fn an_edit_in_the_inspector_changes_the_page_its_policy_aside() {
    let server = net::test_server::TestServer::plain(|_| {
        net::test_server::Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/html"),
                ("Content-Security-Policy", "style-src 'none'"),
            ],
            b"<p style='color: red'>x</p>",
        )
    });
    let mut tab = Tab::new();
    tab.load(server.url("/"));
    let p = node_of(&tab, "p");
    let color = |tab: &mut Tab| {
        let node = NodeId::from_bits(p).expect("a node");
        let styles = tab.engine.styles();
        style::serialize::computed_value(styles.get(node).expect("a style"), "color")
    };
    // The page's policy refuses its own style attribute.
    assert_eq!(color(&mut tab).as_deref(), Some("rgb(0, 0, 0)"));
    // Reading the tree asks to be told of changes.
    ask(&mut tab, "dom.tree", Json::object([])).expect("a tree");
    let edit = devtools::Request {
        id: 2,
        method: "css.setStyle".to_owned(),
        params: Json::object([("node", Json::from(p)), ("text", Json::from("color: blue"))]),
    };
    tab.engine.inspect(&edit.write());
    assert_eq!(color(&mut tab).as_deref(), Some("rgb(0, 0, 255)"));
    let updated = tab.engine.take_inspected();
    assert_eq!(
        updated,
        [
            r#"{"id":2,"result":null}"#,
            r#"{"event":"dom.updated","params":{}}"#
        ]
    );
    // Told once.
    assert!(tab.engine.take_inspected().is_empty());
    let set = Json::object([
        ("node", Json::from(p)),
        ("name", Json::from("data-x")),
        ("value", Json::from("1")),
    ]);
    ask(&mut tab, "dom.setAttribute", set).expect("set");
    let node = NodeId::from_bits(p).expect("a node");
    assert_eq!(tab.engine.shown.document.attr(node, "data-x"), Some("1"));
    let remove = Json::object([("node", Json::from(p)), ("name", Json::from("data-x"))]);
    ask(&mut tab, "dom.removeAttribute", remove).expect("removed");
    assert_eq!(tab.engine.shown.document.attr(node, "data-x"), None);
    let bad = Json::object([
        ("node", Json::from(p)),
        ("name", Json::from("a b")),
        ("value", Json::from("1")),
    ]);
    assert!(ask(&mut tab, "dom.setAttribute", bad).is_err());
}

#[test]
fn the_node_selected_is_highlighted_in_the_page_and_is_dollar_zero() {
    let (mut tab, _server) = inspected(INSPECTED);
    let d = node_of(&tab, "div");
    let before = tab
        .engine
        .frame_at(320, 240, crate::FrameTime::at(std::time::Duration::ZERO))
        .to_vec();
    ask(
        &mut tab,
        "dom.select",
        Json::object([("node", Json::from(d))]),
    )
    .expect("selected");
    assert!(tab.engine.needs_frame());
    let after = tab
        .engine
        .frame_at(320, 240, crate::FrameTime::at(std::time::Duration::ZERO))
        .to_vec();
    // The white div, 100 by 50 at the body's margin, is blue now.
    let pixel = |frame: &[u8], x: usize, y: usize| {
        let at = (y * 320 + x) * 4;
        [frame[at], frame[at + 1], frame[at + 2]]
    };
    let (x, y) = (8 + 50, tab_div_top(&mut tab) + 25);
    assert_eq!(pixel(&before, x, y), [255, 255, 255]);
    assert_eq!(pixel(&after, x, y), [159, 197, 231]);
    // Nothing else is.
    assert_eq!(pixel(&after, 300, 230), pixel(&before, 300, 230));
    // `$0` is the node selected.
    let answer = ask(
        &mut tab,
        "console.evaluate",
        Json::object([("expression", Json::from("$0.id"))]),
    )
    .expect("evaluated");
    assert_eq!(
        answer
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(Json::as_str),
        Some("d")
    );
    // The highlight goes when asked.
    ask(
        &mut tab,
        "dom.highlight",
        Json::object([("node", Json::Null)]),
    )
    .expect("cleared");
    let cleared = tab
        .engine
        .frame_at(320, 240, crate::FrameTime::at(std::time::Duration::ZERO))
        .to_vec();
    assert_eq!(pixel(&cleared, x, y), [255, 255, 255]);
}

/// Where the div's top is in the viewport.
fn tab_div_top(tab: &mut Tab) -> usize {
    let document = &tab.engine.shown.document;
    let div = document
        .subtree(document.root())
        .find(|&node| document.tag(node) == Some("div"))
        .expect("a div");
    tab.engine.border_box(div).expect("a box").0.y as usize
}

#[test]
fn a_pick_selects_what_is_under_the_pointer_and_the_page_hears_nothing() {
    let (mut tab, _server) = inspected(INSPECTED);
    let d = node_of(&tab, "div");
    ask(
        &mut tab,
        "dom.pick",
        Json::object([("on", Json::from(true))]),
    )
    .expect("picking");
    let y = tab_div_top(&mut tab) as f32 + 10.0;
    tab.engine.pointer_move(20.0, y);
    assert_eq!(tab.engine.highlighted.map(NodeId::to_bits), Some(d));
    tab.engine.pointer_down(20.0, y, crate::Button::Primary, 1);
    tab.engine.pointer_up(20.0, y, crate::Button::Primary, 1);
    tab.engine.run_tasks();
    assert_eq!(tab.engine.title(), "", "the page heard nothing");
    let picked = tab.engine.take_inspected();
    let event = Json::parse(&picked[0]).expect("JSON");
    assert_eq!(
        event.get("event").and_then(Json::as_str),
        Some("dom.picked")
    );
    let params = event.get("params").expect("params");
    assert_eq!(params.get("node").and_then(Json::as_u64), Some(d));
    let path = params.get("path").and_then(Json::as_array).expect("a path");
    assert_eq!(path.len(), 3, "the document, html, body");
    // Picking is over: a click is the page's again.
    tab.engine.pointer_down(20.0, y, crate::Button::Primary, 1);
    tab.engine.pointer_up(20.0, y, crate::Button::Primary, 1);
    tab.engine.run_tasks();
    assert_eq!(tab.engine.title(), "duc");
    // A navigation lets go of the nodes.
    tab.load(Url::parse("data:text/html,<p>other").expect("a URL"));
    assert_eq!(tab.engine.selected, None);
    assert_eq!(tab.engine.highlighted, None);
}
