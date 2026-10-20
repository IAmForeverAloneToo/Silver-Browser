//! The DevTools' ends in an engine (M36): the panel's page and its
//! bridge, which no other page has, and a page answering the panel.

use devtools::{Json, Request};

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
    let request = Request::parse(&messages[0]).expect("a request");
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
