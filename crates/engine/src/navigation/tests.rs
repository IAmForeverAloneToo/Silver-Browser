//! The session history (docs/design/navigation.md, "Tests"): the list
//! and its cache on their own, then the engine navigating, going back
//! and forward, reloading and stopping against a scripted server on
//! 127.0.0.1.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use net::test_server::{Reply, TestServer};

use super::{History, MAX_ENTRIES, MAX_KEPT};
use crate::testing::*;
use crate::{Engine, LoadState, Url};

// ---- The list and its cache -----------------------------------------------

fn at(path: &str) -> Option<Url> {
    Some(Url::parse(&format!("https://example.com/{path}")).unwrap())
}

fn entry_paths(history: &History<&'static str>) -> Vec<String> {
    history
        .entries
        .iter()
        .map(|entry| entry.url.as_ref().unwrap().path_str().to_owned())
        .collect()
}

#[test]
fn a_push_drops_the_entries_after_the_current_one() {
    let mut history = History::<&str>::new(at("a"));
    for path in ["b", "c"] {
        let state = history.new_state();
        history.push(at(path), state);
    }
    assert_eq!(entry_paths(&history), ["/a", "/b", "/c"]);
    assert!(history.can_go_back() && !history.can_go_forward());
    let (index, entry) = history.by_delta(-2).unwrap();
    assert_eq!((index, entry.url.clone()), (0, at("a")));
    assert!(history.by_delta(-3).is_none() && history.by_delta(1).is_none());
    history.go_to(1);
    assert!(history.can_go_back() && history.can_go_forward());
    let state = history.new_state();
    history.push(at("d"), state);
    assert_eq!(entry_paths(&history), ["/a", "/b", "/d"]);
    assert_eq!(history.current_index(), 2);
}

#[test]
fn a_replace_puts_a_new_entry_in_the_current_ones_place() {
    let mut history = History::<&str>::new(at("a"));
    let state = history.new_state();
    history.push(at("b"), state);
    history.current_mut().scroll = 300.0;
    let replacing = history.new_state();
    history.replace(at("c"), replacing);
    assert_eq!(entry_paths(&history), ["/a", "/c"]);
    assert_eq!(history.current().state, replacing);
    assert_eq!(history.current().scroll, 0.0);
}

#[test]
fn the_history_holds_fifty_entries_and_drops_the_oldest() {
    let mut history = History::<&str>::new(at("0"));
    for n in 1..=60 {
        let state = history.new_state();
        history.push(at(&n.to_string()), state);
    }
    assert_eq!(history.len(), MAX_ENTRIES);
    assert_eq!(history.current_index(), MAX_ENTRIES - 1);
    assert_eq!(entry_paths(&history)[0], "/11");
    assert_eq!(entry_paths(&history)[MAX_ENTRIES - 1], "/60");
}

#[test]
fn the_cache_keeps_six_documents_and_drops_the_one_shown_least_recently() {
    let mut history = History::<&str>::new(at("0"));
    let names = [
        "zero", "one", "two", "three", "four", "five", "six", "seven",
    ];
    let mut gone = Vec::new();
    for n in 1..names.len() {
        // Leaving entry n - 1 for entry n keeps n - 1's document.
        let left = history.current().state;
        let state = history.new_state();
        history.push(at(&n.to_string()), state);
        gone.extend(history.keep(left, names[n - 1]));
    }
    assert_eq!(history.kept_len(), MAX_KEPT);
    assert_eq!(gone, ["zero"]);
    assert!(!history.is_kept(0));
    // Taking one out and keeping it again makes it the most recent.
    assert_eq!(history.take_kept(1), Some("one"));
    let current = history.current().state;
    assert_eq!(history.keep(1, "one"), Vec::<&str>::new());
    assert_eq!(history.keep(current, "seven"), ["two"]);
    assert!(history.is_kept(1) && !history.is_kept(2));
}

#[test]
fn a_document_no_entry_names_is_not_kept() {
    let mut history = History::<&str>::new(at("a"));
    let a = history.current().state;
    let b = history.new_state();
    history.push(at("b"), b);
    assert!(history.keep(a, "a").is_empty());
    // Going back to a and pushing c drops b's entry, and b with it.
    history.go_to(0);
    assert!(history.keep(b, "b").is_empty() && history.is_kept(b));
    let c = history.new_state();
    history.push(at("c"), c);
    assert!(!history.is_kept(b));
    // A state no entry has is refused outright.
    assert_eq!(history.keep(99, "stray"), ["stray"]);
}

#[test]
fn entries_share_a_state_and_its_document() {
    let mut history = History::<&str>::new(at("a"));
    let a = history.current().state;
    history.push(at("a#x"), a);
    assert!(history.names(a));
    let b = history.new_state();
    history.replace(at("b"), b);
    // The first entry still has a's state, so a's document is kept.
    assert!(history.keep(a, "a").is_empty());
    assert!(history.is_kept(a));
}

// ---- The engine -------------------------------------------------------------

/// A page titled `name`, tall enough to scroll in a 200 by 100 viewport.
fn page(name: &str) -> Reply {
    let paragraphs: String = (0..40).map(|n| format!("<p>{name} {n}</p>")).collect();
    html(&format!("<title>{name}</title><body>{paragraphs}</body>"))
}

/// A server of pages at `/a`, `/b` and so on, each named for its path.
fn pages() -> TestServer {
    serve(|path| {
        let name = path.strip_prefix('/')?;
        (name.len() <= 2 && name.chars().all(|c| c.is_ascii_alphanumeric())).then(|| page(name))
    })
}

/// How many times the server was asked for `path`.
fn asked(server: &TestServer, path: &str) -> usize {
    paths(server).iter().filter(|p| *p == path).count()
}

/// The first paragraph's text: which page is shown.
fn shown(engine: &Engine) -> String {
    let document = engine.document();
    document
        .subtree(document.root())
        .find(|&node| document.tag(node) == Some("p"))
        .map(|node| document.text_content(node))
        .unwrap_or_default()
}

fn at_path(engine: &Engine) -> Option<String> {
    engine.url().map(|url| url.path_str().to_owned())
}

#[test]
fn the_first_navigation_replaces_the_initial_about_blank() {
    let server = pages();
    let mut tab = Tab::new();
    tab.load(server.url("/a"));
    assert_eq!(shown(&tab.engine), "a 0");
    assert_eq!(at_path(&tab.engine).as_deref(), Some("/a"));
    assert!(!tab.engine.can_go_back());
    assert_eq!(tab.engine.history.len(), 1);
}

#[test]
fn navigations_push_entries_and_back_and_forward_show_the_documents_kept() {
    let server = pages();
    let mut tab = Tab::new();
    for path in ["/a", "/b", "/c"] {
        tab.load(server.url(path));
    }
    assert_eq!(tab.engine.history.len(), 3);
    assert!(tab.engine.can_go_back() && !tab.engine.can_go_forward());
    let asked_before = paths(&server).len();

    tab.engine.go_back();
    assert_eq!(shown(&tab.engine), "b 0");
    assert_eq!(at_path(&tab.engine).as_deref(), Some("/b"));
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    tab.engine.go_back();
    assert_eq!(shown(&tab.engine), "a 0");
    assert!(!tab.engine.can_go_back() && tab.engine.can_go_forward());
    // Past the first entry, nothing happens.
    tab.engine.go_back();
    assert_eq!(shown(&tab.engine), "a 0");
    tab.engine.go_forward();
    tab.engine.go_forward();
    assert_eq!(shown(&tab.engine), "c 0");
    tab.engine.go_forward();
    assert_eq!(shown(&tab.engine), "c 0");
    // Every document came from the cache: the server was not asked.
    assert_eq!(paths(&server).len(), asked_before);
}

#[test]
fn a_document_from_the_cache_comes_back_whole_with_its_layout() {
    let server = pages();
    let mut tab = Tab::new();
    tab.load(server.url("/a"));
    let before = tab.engine.frame(200, 100);
    tab.load(server.url("/b"));
    tab.engine.frame(200, 100);
    let (styles, layouts) = (tab.engine.style_runs(), tab.engine.layout_runs());
    tab.engine.go_back();
    assert!(tab.engine.needs_frame());
    assert_eq!(tab.engine.frame(200, 100), before);
    assert_eq!(tab.engine.style_runs(), styles, "no cascade");
    assert_eq!(tab.engine.layout_runs(), layouts, "no layout");
}

#[test]
fn a_document_shown_in_place_of_another_is_drawn_whole() {
    let server = serve(|path| match path {
        "/a" => Some(html("<p>a</p><p>the same</p>")),
        "/b" => Some(html("<p>b</p><p>the same</p>")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/a"));
    tab.engine.frame(200, 100);
    tab.load(server.url("/b"));
    tab.engine.frame(200, 100);
    assert_eq!(tab.engine.redrawn(), &render::Redrawn::Whole);
    tab.engine.frame(200, 100);
    assert_eq!(tab.engine.redrawn(), &render::Redrawn::Rects(Vec::new()));
    // Not compared with the frame of the document it replaces, however
    // little of it differs.
    tab.engine.go_back();
    tab.engine.frame(200, 100);
    assert_eq!(tab.engine.redrawn(), &render::Redrawn::Whole);
}

#[test]
fn a_navigation_to_the_url_shown_replaces_its_entry() {
    let server = pages();
    let mut tab = Tab::new();
    tab.load(server.url("/a"));
    tab.load(server.url("/b"));
    tab.load(server.url("/b"));
    assert_eq!(tab.engine.history.len(), 2);
    assert_eq!(
        asked(&server, "/b"),
        2,
        "fetched again, not taken from the cache"
    );
    tab.engine.go_back();
    assert_eq!(shown(&tab.engine), "a 0");
}

#[test]
fn a_navigation_from_an_earlier_entry_drops_the_entries_after_it() {
    let server = pages();
    let mut tab = Tab::new();
    for path in ["/a", "/b", "/c"] {
        tab.load(server.url(path));
    }
    tab.engine.go_back();
    tab.engine.go_back();
    tab.load(server.url("/d"));
    assert_eq!(tab.engine.history.len(), 2);
    assert!(!tab.engine.can_go_forward());
    tab.engine.go_back();
    assert_eq!(shown(&tab.engine), "a 0");
    tab.engine.go_forward();
    assert_eq!(shown(&tab.engine), "d 0");
}

#[test]
fn the_cache_keeps_six_documents_and_the_seventh_left_drops_the_oldest() {
    let server = pages();
    let mut tab = Tab::new();
    let names: Vec<String> = (0..8).map(|n| format!("p{n}")).collect();
    for name in &names {
        tab.load(server.url(&format!("/{name}")));
    }
    // p0 to p6 were left, and the cache keeps the last six of them.
    assert_eq!(tab.engine.history.kept_len(), MAX_KEPT);
    for name in names[..7].iter().rev() {
        tab.engine.go_back();
        tab.settle();
        assert_eq!(shown(&tab.engine), format!("{name} 0"));
    }
    for name in &names[1..] {
        assert_eq!(
            asked(&server, &format!("/{name}")),
            1,
            "{name} came from the cache"
        );
    }
    assert_eq!(
        asked(&server, "/p0"),
        2,
        "p0 was dropped, and fetched again"
    );
}

#[test]
fn a_document_left_while_a_stylesheet_is_on_its_way_is_not_kept() {
    let gate = Gate::new();
    let server = {
        let gate = gate.clone();
        serve(move |path| match path {
            "/slow" => Some(html(
                "<body><p>slow 0</p><link rel=stylesheet href=slow.css></body>",
            )),
            "/slow.css" => {
                gate.wait();
                Some(css("p { color: #f00 }"))
            }
            "/b" => Some(page("b")),
            _ => None,
        })
    };
    let mut tab = Tab::new();
    tab.engine.navigate(server.url("/slow"));
    tab.wait_until(|engine| engine.navigation.is_none());
    assert_eq!(tab.engine.load_state(), &LoadState::Loading);
    tab.load(server.url("/b"));
    gate.open();
    assert_eq!(tab.engine.history.kept_len(), 0);
    tab.engine.go_back();
    tab.wait_until(|engine| engine.navigation.is_none());
    assert_eq!(shown(&tab.engine), "slow 0");
    assert_eq!(asked(&server, "/slow"), 2);
}

#[test]
fn scroll_positions_are_saved_and_restored_on_back_and_forward() {
    let server = pages();
    let mut tab = Tab::new();
    tab.load(server.url("/a"));
    tab.engine.frame(200, 100);
    tab.engine.set_scroll(300.0);
    tab.engine.frame(200, 100);
    tab.load(server.url("/b"));
    tab.engine.frame(200, 100);
    assert_eq!(
        tab.engine.scroll_y(),
        0.0,
        "a new document starts at the top"
    );
    tab.engine.set_scroll(120.0);
    tab.engine.frame(200, 100);
    tab.engine.go_back();
    tab.engine.frame(200, 100);
    assert_eq!(tab.engine.scroll_y(), 300.0);
    tab.engine.go_forward();
    tab.engine.frame(200, 100);
    assert_eq!(tab.engine.scroll_y(), 120.0);
}

#[test]
fn a_document_fetched_again_is_shown_at_its_entrys_scroll_position() {
    let server = pages();
    let mut tab = Tab::new();
    for name in (0..8).map(|n| format!("/p{n}")) {
        tab.load(server.url(&name));
        tab.engine.frame(200, 100);
        if name == "/p0" {
            tab.engine.set_scroll(250.0);
            tab.engine.frame(200, 100);
        }
    }
    for _ in 0..7 {
        tab.engine.go_back();
        tab.settle();
    }
    assert_eq!(asked(&server, "/p0"), 2);
    tab.engine.frame(200, 100);
    assert_eq!(tab.engine.scroll_y(), 250.0);
}

#[test]
fn a_restored_position_is_given_up_once_the_user_scrolls() {
    let server = pages();
    let mut tab = Tab::new();
    tab.load(server.url("/a"));
    tab.engine.frame(200, 100);
    tab.engine.set_scroll(300.0);
    tab.engine.frame(200, 100);
    tab.load(server.url("/b"));
    tab.engine.go_back();
    // The user scrolls before the frame that would restore the position.
    tab.engine.set_scroll(40.0);
    tab.engine.frame(200, 100);
    assert_eq!(tab.engine.scroll_y(), 40.0);
}

#[test]
fn reload_fetches_the_entry_again_and_keeps_its_place() {
    let server = pages();
    let mut tab = Tab::new();
    tab.load(server.url("/a"));
    tab.load(server.url("/b"));
    tab.engine.frame(200, 100);
    tab.engine.set_scroll(200.0);
    tab.engine.frame(200, 100);
    tab.engine.reload();
    assert_eq!(tab.engine.load_state(), &LoadState::Loading);
    tab.settle();
    assert_eq!(asked(&server, "/b"), 2);
    assert_eq!(tab.engine.history.len(), 2);
    // The document reloaded is gone, not kept beside its successor.
    assert_eq!(tab.engine.history.kept_len(), 1);
    tab.engine.frame(200, 100);
    assert_eq!(tab.engine.scroll_y(), 200.0);
    tab.engine.go_back();
    assert_eq!(shown(&tab.engine), "a 0");
    assert_eq!(asked(&server, "/a"), 1, "the entry before was untouched");
}

#[test]
fn stop_ends_the_load_and_the_document_is_not_kept() {
    let gate = Gate::new();
    let server = {
        let gate = gate.clone();
        serve(move |path| match path {
            "/slow" => Some(html(
                "<body><p>slow 0</p><link rel=stylesheet href=slow.css></body>",
            )),
            "/slow.css" => {
                gate.wait();
                Some(css("p { color: #f00 }"))
            }
            "/b" => Some(page("b")),
            _ => None,
        })
    };
    let mut tab = Tab::new();
    tab.engine.navigate(server.url("/slow"));
    tab.wait_until(|engine| engine.navigation.is_none());
    assert_eq!(tab.engine.load_state(), &LoadState::Loading);
    tab.engine.stop();
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    assert_eq!(tab.engine.network.in_flight(), 0);
    gate.open();
    tab.load(server.url("/b"));
    assert_eq!(
        tab.engine.history.kept_len(),
        0,
        "a stopped document is not kept"
    );
    tab.engine.go_back();
    tab.wait_until(|engine| engine.navigation.is_none());
    assert_eq!(asked(&server, "/slow"), 2);
}

#[test]
fn stop_aborts_the_navigation_under_way_and_the_page_shown_stays() {
    let gate = Gate::new();
    let server = {
        let gate = gate.clone();
        serve(move |path| match path {
            "/a" => Some(page("a")),
            "/held" => {
                gate.wait();
                Some(page("held"))
            }
            _ => None,
        })
    };
    let mut tab = Tab::new();
    tab.load(server.url("/a"));
    tab.engine.navigate(server.url("/held"));
    assert_eq!(tab.engine.load_state(), &LoadState::Loading);
    tab.engine.stop();
    gate.open();
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    assert!(tab.engine.navigation.is_none());
    tab.engine.poll_network();
    assert_eq!(shown(&tab.engine), "a 0");
    assert_eq!(tab.engine.history.len(), 1);
}

#[test]
fn a_traversal_aborts_the_navigation_under_way() {
    let gate = Gate::new();
    let server = {
        let gate = gate.clone();
        serve(move |path| match path {
            "/a" => Some(page("a")),
            "/b" => Some(page("b")),
            "/held" => {
                gate.wait();
                Some(page("held"))
            }
            _ => None,
        })
    };
    let mut tab = Tab::new();
    tab.load(server.url("/a"));
    tab.load(server.url("/b"));
    tab.engine.navigate(server.url("/held"));
    tab.engine.go_back();
    gate.open();
    tab.engine.poll_network();
    assert!(tab.engine.navigation.is_none());
    assert_eq!(shown(&tab.engine), "a 0");
    assert_eq!(tab.engine.history.len(), 2);
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
}

#[test]
fn a_document_given_to_the_engine_starts_a_new_history() {
    let server = pages();
    let mut tab = Tab::new();
    tab.load(server.url("/a"));
    tab.load(server.url("/b"));
    tab.engine.load_document("<p>given</p>");
    assert_eq!(tab.engine.url(), None);
    assert!(!tab.engine.can_go_back());
    assert_eq!(tab.engine.history.len(), 1);
    assert_eq!(tab.engine.history.kept_len(), 0);
    // Nothing to fetch again: a reload of it does nothing.
    tab.engine.reload();
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    assert_eq!(shown(&tab.engine), "given");
}

#[test]
fn a_traversal_to_a_document_fetched_again_takes_the_url_it_redirects_to() {
    let moved = Arc::new(AtomicBool::new(false));
    let server = {
        let moved = moved.clone();
        serve(move |path| match path {
            "/moving" if moved.load(Ordering::SeqCst) => {
                Some(Reply::with_fields(301, &[("Location", "/moved")], b""))
            }
            "/moving" => Some(page("moving")),
            "/moved" => Some(page("moved")),
            name => name
                .strip_prefix("/p")
                .filter(|n| n.parse::<u8>().is_ok())
                .map(|n| page(&format!("p{n}"))),
        })
    };
    let mut tab = Tab::new();
    tab.load(server.url("/moving"));
    // Seven documents left after it push it out of the cache.
    for n in 1..=7 {
        tab.load(server.url(&format!("/p{n}")));
    }
    moved.store(true, Ordering::SeqCst);
    for _ in 0..7 {
        tab.engine.go_back();
        tab.settle();
    }
    assert_eq!(shown(&tab.engine), "moved 0");
    assert_eq!(at_path(&tab.engine).as_deref(), Some("/moved"));
    assert!(!tab.engine.can_go_back() && tab.engine.can_go_forward());
}

// ---- What a response becomes ----------------------------------------------

/// The text of the document shown.
fn all_text(engine: &Engine) -> String {
    text(engine)
}

/// The tags of the document's elements, in tree order.
fn tags(engine: &Engine) -> Vec<String> {
    let document = engine.document();
    document
        .subtree(document.root())
        .filter_map(|node| document.tag(node).map(str::to_owned))
        .collect()
}

/// The `<title>`'s text, as the browser's pages set it.
fn title_text(engine: &Engine) -> String {
    let document = engine.document();
    document
        .subtree(document.root())
        .find(|&node| document.tag(node) == Some("title"))
        .map(|node| document.text_content(node))
        .unwrap_or_default()
}

fn failed(engine: &Engine) -> bool {
    matches!(engine.load_state(), LoadState::Failed(_))
}

#[test]
fn text_types_are_shown_as_text_documents() {
    let server = serve(|path| {
        let (kind, body): (&str, &[u8]) = match path {
            "/plain" => ("text/plain", b"a <b>bold</b> & more\r\nnext\rlast"),
            "/json" => ("application/json", b"{\"a\": [1, 2]}"),
            "/script" => ("text/javascript", b"let a = 1 < 2;"),
            // No sniffing for CSS: a NUL stays, and is replaced.
            "/style" => ("text/css", b"p { color: red }\0"),
            "/feed" => ("application/rss+xml", b"<rss><channel/></rss>"),
            "/captions" => ("text/vtt", b"WEBVTT"),
            "/latin" => ("text/plain;charset=windows-1252", b"caf\xe9"),
            _ => return None,
        };
        Some(Reply::ok(kind, body))
    });
    let mut tab = Tab::new();
    tab.load(server.url("/plain"));
    assert_eq!(tags(&tab.engine), ["html", "head", "style", "body", "pre"]);
    let document = tab.engine.document();
    let pre = document
        .subtree(document.root())
        .find(|&node| document.tag(node) == Some("pre"))
        .unwrap();
    assert_eq!(
        document.text_content(pre),
        "a <b>bold</b> & more\nnext\nlast"
    );
    // HTML lets the user agent add a style: a long line wraps.
    let white_space = tab.engine.styles().get(pre).unwrap().white_space;
    assert_eq!(white_space, style::computed::WhiteSpace::PreWrap);
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);

    for (path, shown) in [
        ("/json", "{\"a\": [1, 2]}"),
        ("/script", "let a = 1 < 2;"),
        ("/style", "p { color: red }\u{FFFD}"),
        ("/feed", "<rss><channel/></rss>"),
        ("/captions", "WEBVTT"),
        ("/latin", "café"),
    ] {
        tab.load(server.url(path));
        assert!(tags(&tab.engine).ends_with(&["pre".to_owned()]), "{path}");
        assert!(all_text(&tab.engine).ends_with(shown), "{path}");
    }
    assert_eq!(tab.engine.history.len(), 7);
}

#[test]
fn xhtml_goes_through_the_html_parser() {
    let server = serve(|path| {
        (path == "/page.xhtml").then(|| {
            Reply::ok(
                "application/xhtml+xml",
                b"<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>XHTML</p></body></html>",
            )
        })
    });
    let mut tab = Tab::new();
    tab.load(server.url("/page.xhtml"));
    assert_eq!(shown(&tab.engine), "XHTML");
}

#[test]
fn a_204_or_a_205_leaves_the_page_and_the_history_as_they_were() {
    let server = serve(|path| match path {
        "/a" => Some(page("a")),
        "/none" => Some(Reply::with_fields(204, &[], b"")),
        "/reset" => Some(Reply::with_fields(205, &[], b"")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/a"));
    for path in ["/none", "/reset"] {
        tab.load(server.url(path));
        assert_eq!(shown(&tab.engine), "a 0", "{path}");
        assert_eq!(at_path(&tab.engine).as_deref(), Some("/a"));
        assert_eq!(tab.engine.history.len(), 1);
        assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
        assert_eq!(tab.engine.take_notice(), None);
    }
}

#[test]
fn a_download_leaves_the_page_shown_with_a_notice() {
    let server = serve(|path| match path {
        "/a" => Some(page("a")),
        "/report" => Some(Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/html"),
                ("Content-Disposition", " ATTACHMENT; filename=report.html"),
            ],
            b"<p>report</p>",
        )),
        "/archive" => Some(Reply::ok("application/zip", b"PK\x03\x04")),
        "/paper" => Some(Reply::ok("application/pdf", b"%PDF-1.7")),
        "/inline" => Some(Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/html"),
                ("Content-Disposition", "inline"),
            ],
            b"<p>inline 0</p>",
        )),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/a"));
    for (path, essence) in [
        ("/report", "text/html"),
        ("/archive", "application/zip"),
        ("/paper", "application/pdf"),
    ] {
        tab.load(server.url(path));
        assert_eq!(shown(&tab.engine), "a 0", "{path}");
        assert_eq!(tab.engine.history.len(), 1);
        assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
        let notice = tab.engine.take_notice().expect("a notice");
        assert!(
            notice.contains(path) && notice.contains(essence),
            "{notice}"
        );
        assert_eq!(tab.engine.take_notice(), None, "told once");
    }
    // Any other disposition shows the document.
    tab.load(server.url("/inline"));
    assert_eq!(shown(&tab.engine), "inline 0");
}

#[test]
fn an_image_is_a_page_saying_it_cannot_be_shown_yet() {
    let server = serve(|path| match path {
        "/a" => Some(page("a")),
        "/photo" => Some(Reply::ok("image/png", b"\x89PNG\r\n\x1a\n")),
        "/song" => Some(Reply::ok("audio/ogg", b"OggS")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/a"));
    tab.load(server.url("/photo"));
    assert_eq!(at_path(&tab.engine).as_deref(), Some("/photo"));
    assert!(all_text(&tab.engine).contains("This is a image/png file"));
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    tab.load(server.url("/song"));
    assert!(all_text(&tab.engine).contains("audio/ogg"));
    // A page the browser made is asked for again, not kept.
    tab.engine.go_back();
    tab.settle();
    assert_eq!(asked(&server, "/photo"), 2);
}

#[test]
fn an_error_status_is_the_sites_page_unless_it_sent_nothing() {
    let server = serve(|path| match path {
        "/missing" => Some(Reply::with_fields(
            404,
            &[("Content-Type", "text/html")],
            b"<p>Not here</p>",
        )),
        "/empty" => Some(Reply::Send(
            b"HTTP/1.1 503 <b>Busy</b> & away\r\nContent-Length: 0\r\n\r\n".to_vec(),
        )),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/missing"));
    assert_eq!(all_text(&tab.engine), "Not here");
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    tab.load(server.url("/empty"));
    assert_eq!(at_path(&tab.engine).as_deref(), Some("/empty"));
    assert!(all_text(&tab.engine).contains("HTTP 503 <b>Busy</b> & away"));
    assert!(
        !tags(&tab.engine).contains(&"b".to_owned()),
        "text, not markup"
    );
    assert_eq!(
        title_text(&tab.engine),
        "127.0.0.1 — HTTP 503 <b>Busy</b> & away"
    );
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
}

#[test]
fn a_network_error_shows_an_error_page_under_the_url_that_failed() {
    let server = pages();
    let closed_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut tab = Tab::new();
    tab.load(server.url("/a"));

    let nowhere = Url::parse("http://nowhere.invalid/page").unwrap();
    tab.load(nowhere.clone());
    assert_eq!(tab.engine.url(), Some(&nowhere));
    assert!(matches!(
        tab.engine.load_state(),
        LoadState::Failed(crate::LoadError::Network(crate::NetworkError::Resolve(_)))
    ));
    assert_eq!(title_text(&tab.engine), "Can't find nowhere.invalid");
    assert!(all_text(&tab.engine).contains("http://nowhere.invalid/page"));
    assert_eq!(tab.engine.history.len(), 2, "an entry like any other");

    let refused = Url::parse(&format!("http://127.0.0.1:{closed_port}/")).unwrap();
    tab.load(refused.clone());
    assert!(failed(&tab.engine));
    assert_eq!(title_text(&tab.engine), "Can't reach 127.0.0.1");

    let file = Url::parse("file:///no/such/file/anywhere.html").unwrap();
    tab.load(file);
    assert_eq!(title_text(&tab.engine), "Can't open this file");

    let bad = Url::parse("data:text/html;base64,%%%").unwrap();
    tab.load(bad);
    assert!(failed(&tab.engine), "{:?}", tab.engine.load_state());

    // Back leaves an error page; one gone back to is fetched again, and
    // fails again; the entry before them is as it was.
    tab.engine.go_back();
    tab.settle();
    tab.engine.go_back();
    tab.settle();
    assert_eq!(tab.engine.url(), Some(&refused));
    assert!(failed(&tab.engine));
    for _ in 0..2 {
        tab.engine.go_back();
        tab.settle();
    }
    assert_eq!(shown(&tab.engine), "a 0");
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
}

#[test]
fn a_certificate_from_a_root_not_trusted_is_an_error_with_no_way_past() {
    let authority = net::test_server::TestAuthority::new();
    let server = TestServer::tls(
        authority.server(&["127.0.0.1"], net::test_server::VALID),
        |_| page("secret"),
    );
    let mut tab = Tab::new();
    tab.load(server.url("/"));
    assert!(matches!(
        tab.engine.load_state(),
        LoadState::Failed(crate::LoadError::Network(crate::NetworkError::Tls(
            net::TlsFailure::UnknownIssuer
        )))
    ));
    assert_eq!(
        title_text(&tab.engine),
        "Can't connect securely to 127.0.0.1"
    );
    let said = all_text(&tab.engine);
    assert!(
        said.contains("not issued by an authority this browser trusts"),
        "{said}"
    );
    assert!(said.contains("there is no way past this"), "{said}");
    assert!(
        !tags(&tab.engine).contains(&"a".to_owned()),
        "nothing to click through"
    );
}

#[test]
fn a_reload_of_an_error_page_tries_the_url_again() {
    let up = Arc::new(AtomicBool::new(false));
    let server = {
        let up = up.clone();
        serve(move |path| match path {
            "/flaky" if up.load(Ordering::SeqCst) => Some(page("flaky")),
            "/flaky" => Some(Reply::Close),
            _ => None,
        })
    };
    let mut tab = Tab::new();
    tab.load(server.url("/flaky"));
    assert!(failed(&tab.engine));
    up.store(true, Ordering::SeqCst);
    tab.engine.reload();
    tab.settle();
    assert_eq!(shown(&tab.engine), "flaky 0");
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    assert_eq!(tab.engine.history.len(), 1);
}

#[test]
fn stopping_on_an_error_page_leaves_it_failed() {
    let server = serve(|path| (path == "/gone").then_some(Reply::Close));
    let mut tab = Tab::new();
    tab.load(server.url("/gone"));
    assert!(failed(&tab.engine));
    tab.engine.stop();
    assert!(failed(&tab.engine));
}

#[test]
fn about_silver_is_the_browsers_own_page_and_fetches_nothing() {
    let mut tab = Tab::new();
    let about = Url::parse("about:silver").unwrap();
    tab.engine.navigate(about.clone());
    // Made at once, with nothing to wait for.
    assert_eq!(tab.engine.url(), Some(&about));
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    assert!(all_text(&tab.engine).contains("Hello from Silver Browser"));
    assert_eq!(tab.engine.network.in_flight(), 0);
    assert!(
        !tab.engine.can_go_back(),
        "it replaced the initial about:blank"
    );
    tab.engine.navigate(about.clone());
    assert_eq!(
        tab.engine.history.len(),
        1,
        "a navigation to it again replaces it"
    );
    tab.engine.reload();
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    assert!(all_text(&tab.engine).contains("Hello from Silver Browser"));
}

#[test]
fn about_blank_is_empty_and_any_other_about_page_is_an_error() {
    let mut tab = Tab::new();
    tab.engine.navigate(Url::parse("about:silver").unwrap());
    tab.load(Url::parse("about:blank").unwrap());
    assert_eq!(all_text(&tab.engine), "");
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    assert_eq!(tab.engine.history.len(), 2);
    let nothing = Url::parse("about:nothing").unwrap();
    tab.engine.navigate(nothing.clone());
    assert_eq!(tab.engine.url(), Some(&nothing));
    assert!(failed(&tab.engine));
    assert_eq!(title_text(&tab.engine), "No page at about:nothing");
}

#[test]
fn a_timeout_is_an_error_page() {
    let server = serve(|path| (path == "/slow").then_some(Reply::Hang));
    let mut tab = Tab::new();
    let mut config = net::transport::Config::default();
    config.read_timeout = std::time::Duration::from_millis(200);
    tab.engine.set_network(crate::Client::new(config));
    tab.load(server.url("/slow"));
    assert!(matches!(
        tab.engine.load_state(),
        LoadState::Failed(crate::LoadError::Network(crate::NetworkError::Timeout))
    ));
    assert!(all_text(&tab.engine).contains("took too long to answer"));
}

// ---- Links and fragments ----------------------------------------------------

/// The element whose ID is `id`.
fn element_by_id(engine: &Engine, id: &str) -> dom::NodeId {
    let document = engine.document();
    document
        .subtree(document.root())
        .find(|&node| document.attr(node, "id") == Some(id))
        .unwrap_or_else(|| panic!("no #{id}"))
}

/// A viewport point on the element whose ID is `id`, scrolled to where
/// a viewport of 400 by 100 shows it.
fn point_on(engine: &mut Engine, id: &str) -> (f32, f32) {
    engine.frame(400, 100);
    let node = element_by_id(engine, id);
    let fragment = engine
        .fragments(400.0)
        .into_iter()
        .find(|fragment| fragment.node == node)
        .unwrap_or_else(|| panic!("#{id} is laid out"));
    let top = fragment.top();
    if top < engine.scroll_y() || top + 2.0 > engine.scroll_y() + 100.0 {
        engine.set_scroll(top);
        engine.frame(400, 100);
    }
    (fragment.x + 1.0, top + 1.0 - engine.scroll_y())
}

/// The link on the element whose ID is `id`, found as the pointer finds
/// it.
fn link_on(engine: &mut Engine, id: &str) -> crate::Link {
    let (x, y) = point_on(engine, id);
    engine
        .link_at(x, y)
        .unwrap_or_else(|| panic!("#{id} is a link"))
}

/// Clicks the link on the element whose ID is `id`.
fn click(tab: &mut Tab, id: &str) {
    let link = link_on(&mut tab.engine, id);
    tab.engine.follow_link(&link);
    tab.settle();
}

/// The top of the element whose ID is `id` in the document, after a
/// frame of 400 by 100.
fn top_of(engine: &mut Engine, id: &str) -> f32 {
    engine.frame(400, 100);
    let node = element_by_id(engine, id);
    engine
        .fragments(400.0)
        .into_iter()
        .find(|fragment| fragment.node == node)
        .map(|fragment| fragment.top())
        .unwrap()
}

/// A long page whose paragraphs have IDs `p0` to `p39`, with what
/// `head` holds before them.
fn long_page(head: &str) -> Reply {
    let paragraphs: String = (0..40).map(|n| format!("<p id=p{n}>{n}</p>")).collect();
    html(&format!(
        "{head}<body style=\"margin: 0\">{paragraphs}</body>"
    ))
}

#[test]
fn a_links_url_is_parsed_against_the_documents_base_url_and_encoding() {
    let server = serve(|path| match path {
        "/dir/page.html" => Some(Reply::ok(
            "text/html;charset=windows-1252",
            b"<p><a id=before href=\"other.html?q=\xe9\">before</a></p>\
              <base href=\"/base/\">\
              <p><a id=after href=\"#here\">after</a></p>\
              <p><a id=broken href=\"http://[nope/\">broken</a></p>",
        )),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/dir/page.html"));
    // Parsed when followed, against the first <base> wherever it stands,
    // with the query in the document's encoding.
    let before = link_on(&mut tab.engine, "before");
    assert_eq!(before.url(), &server.url("/base/other.html?q=%E9"));
    assert_eq!(
        link_on(&mut tab.engine, "after").url(),
        &server.url("/base/#here")
    );
    let (x, y) = point_on(&mut tab.engine, "broken");
    assert_eq!(
        tab.engine.link_at(x, y),
        None,
        "an href that does not parse"
    );
}

#[test]
fn a_click_on_a_link_navigates_and_back_comes_back() {
    let server = serve(|path| match path {
        "/a" => Some(html("<p><a id=go href=\"/b\">to b</a></p>")),
        "/b" => Some(page("b")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/a"));
    click(&mut tab, "go");
    assert_eq!(shown(&tab.engine), "b 0");
    assert_eq!(tab.engine.history.len(), 2);
    tab.engine.go_back();
    assert_eq!(at_path(&tab.engine).as_deref(), Some("/a"));
}

#[test]
fn a_page_may_navigate_only_where_the_browsers_let_it() {
    let server = serve(|path| match path {
        "/a" => Some(html(
            "<p><a id=file href=\"file:///etc/hosts\">file</a></p>\
             <p><a id=data href=\"data:text/html,<p>hi\">data</a></p>\
             <p><a id=silver href=\"about:silver\">silver</a></p>\
             <p><a id=script href=\"javascript:void(0)\">script</a></p>\
             <p><a id=mail href=\"mailto:someone@example.com\">mail</a></p>\
             <p><a id=blank href=\"about:blank\">blank</a></p>",
        )),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/a"));
    for (id, said) in [
        ("file", Some("files on this computer")),
        ("data", Some("data: address")),
        ("silver", Some("the browser's own pages")),
        ("script", None),
        ("mail", Some("mailto: links open in another program")),
    ] {
        click(&mut tab, id);
        assert_eq!(at_path(&tab.engine).as_deref(), Some("/a"), "{id}");
        assert_eq!(tab.engine.history.len(), 1, "{id}");
        let notice = tab.engine.take_notice();
        match said {
            Some(said) => assert!(
                notice.as_deref().is_some_and(|n| n.contains(said)),
                "{id}: {notice:?}"
            ),
            None => assert_eq!(notice, None, "{id}"),
        }
    }
    click(&mut tab, "blank");
    assert_eq!(tab.engine.url().map(Url::as_str), Some("about:blank"));
}

#[test]
fn a_file_page_may_open_files_and_the_browsers_page_its_own_pages() {
    let dir = std::env::temp_dir().join(format!("silver-links-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("one.html"),
        "<p><a id=two href=\"two.html\">two</a></p>",
    )
    .unwrap();
    std::fs::write(dir.join("two.html"), "<p>two 0</p>").unwrap();
    let one = Url::from_file_path(&dir.join("one.html")).unwrap();
    let mut tab = Tab::new();
    tab.load(one);
    click(&mut tab, "two");
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(shown(&tab.engine), "two 0");

    tab.engine.navigate(Url::parse("about:silver").unwrap());
    tab.engine.mutate(|document| {
        let body = document
            .subtree(document.root())
            .find(|&node| document.tag(node) == Some("body"))
            .unwrap();
        let link = document.create_element("a").unwrap();
        document
            .set_attribute(link, "href", "about:nothing")
            .unwrap();
        document.set_attribute(link, "id", "own").unwrap();
        let text = document.create_text("own").unwrap();
        document.append_child(link, text).unwrap();
        let first = document.first_child(body);
        document.insert_before(body, link, first).unwrap();
    });
    click(&mut tab, "own");
    assert_eq!(tab.engine.url().map(Url::as_str), Some("about:nothing"));
}

#[test]
fn a_link_whose_document_or_element_is_gone_does_nothing() {
    let server = serve(|path| match path {
        "/a" => Some(html(
            "<p><a id=go href=\"/b\">to b</a></p><p id=other>x</p>",
        )),
        "/b" => Some(page("b")),
        "/c" => Some(page("c")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/a"));
    let link = link_on(&mut tab.engine, "go");
    // Removed from the document: it cannot navigate.
    tab.engine.mutate(|document| {
        let parent = document.parent(link.node()).unwrap();
        document.remove_child(parent, link.node()).unwrap();
    });
    tab.engine.follow_link(&link);
    tab.settle();
    assert_eq!(at_path(&tab.engine).as_deref(), Some("/a"));
    // Its document no longer shown.
    tab.load(server.url("/a"));
    let link = link_on(&mut tab.engine, "go");
    tab.load(server.url("/c"));
    tab.engine.follow_link(&link);
    tab.settle();
    assert_eq!(shown(&tab.engine), "c 0");
    assert_eq!(asked(&server, "/b"), 0);
}

#[test]
fn a_link_is_followed_to_where_its_href_points_when_it_is_followed() {
    let server = serve(|path| match path {
        "/a" => Some(html("<p><a id=go href=\"/b\">go</a></p>")),
        "/b" => Some(page("b")),
        "/c" => Some(page("c")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/a"));
    let link = link_on(&mut tab.engine, "go");
    tab.engine.mutate(|document| {
        document.set_attribute(link.node(), "href", "/c").unwrap();
    });
    tab.engine.follow_link(&link);
    tab.settle();
    assert_eq!(shown(&tab.engine), "c 0");
}

#[test]
fn a_fragment_navigation_fetches_nothing_and_scrolls_to_its_element() {
    let server = serve(|path| match path {
        "/long" => Some(long_page("<p><a id=go href=\"#p30\">far</a></p>")),
        "/b" => Some(page("b")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/long"));
    let far = top_of(&mut tab.engine, "p30");
    assert!(far > 100.0);
    click(&mut tab, "go");
    assert_eq!(asked(&server, "/long"), 1, "nothing fetched");
    assert_eq!(tab.engine.history.len(), 2);
    assert_eq!(tab.engine.url(), Some(&server.url("/long#p30")));
    // The document's URL is its entry's, as HTML's document URL is
    // (scripts will read it, M25).
    assert_eq!(tab.engine.shown.page.url(), tab.engine.url());
    tab.engine.frame(400, 100);
    assert_eq!(tab.engine.scroll_y(), far);

    // Back is within the document: its URL and scroll position return.
    tab.engine.go_back();
    assert_eq!(tab.engine.url(), Some(&server.url("/long")));
    assert_eq!(tab.engine.shown.page.url(), tab.engine.url());
    assert_eq!(tab.engine.document().root(), tab.engine.document().root());
    tab.engine.frame(400, 100);
    assert_eq!(tab.engine.scroll_y(), 0.0);
    // A link now resolves against the document's URL as it stands.
    tab.engine.go_forward();
    tab.engine.frame(400, 100);
    assert_eq!(tab.engine.scroll_y(), far);
    assert_eq!(asked(&server, "/long"), 1);
}

#[test]
fn entries_of_one_document_each_keep_their_own_scroll_position() {
    let server = serve(|path| match path {
        "/long" => Some(long_page("")),
        "/b" => Some(page("b")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/long"));
    tab.load(server.url("/long#p30"));
    tab.engine.frame(400, 100);
    let far = tab.engine.scroll_y();
    assert!(far > 100.0);
    tab.load(server.url("/b"));
    // Two back, past /long#p30, to /long: the document the cache kept is
    // shown at /long's position, not the one it was left at.
    tab.engine.traverse(-2);
    assert_eq!(tab.engine.url(), Some(&server.url("/long")));
    tab.engine.frame(400, 100);
    assert_eq!(tab.engine.scroll_y(), 0.0);
    tab.engine.go_forward();
    tab.engine.frame(400, 100);
    assert_eq!(tab.engine.scroll_y(), far);
    assert_eq!(asked(&server, "/long"), 1);
}

#[test]
fn the_indicated_part_is_found_as_html_finds_it() {
    let server = serve(|path| match path {
        "/page" => Some(html(
            "<body style=\"margin: 0\">\
             <a name=both>name first</a>\
             <div style=\"height: 300px\"></div>\
             <p id=both>id later</p>\
             <div style=\"height: 300px\"></div>\
             <p id=\"\u{e9}t\u{e9}\">encoded</p>\
             <div style=\"height: 300px\"></div>\
             <a name=named>named</a>\
             <div style=\"height: 900px\"></div></body>",
        )),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/page"));
    let both = top_of(&mut tab.engine, "both");
    let encoded = top_of(&mut tab.engine, "\u{e9}t\u{e9}");
    let named = {
        tab.engine.frame(400, 100);
        let document = tab.engine.document();
        let node = document
            .subtree(document.root())
            .find(|&node| document.attr(node, "name") == Some("named"))
            .unwrap();
        tab.engine
            .fragments(400.0)
            .into_iter()
            .find(|fragment| fragment.node == node)
            .unwrap()
            .top()
    };
    for (fragment, expected) in [
        ("both", Some(both)),
        ("%C3%A9t%C3%A9", Some(encoded)),
        ("named", Some(named)),
        ("top", Some(0.0)),
        ("ToP", Some(0.0)),
        ("", Some(0.0)),
        ("nothing", None),
    ] {
        // Start somewhere else, to see the scroll.
        tab.engine.set_scroll(150.0);
        tab.engine.frame(400, 100);
        tab.engine
            .navigate(server.url(&format!("/page#{fragment}")));
        tab.engine.frame(400, 100);
        assert_eq!(
            tab.engine.scroll_y(),
            expected.unwrap_or(150.0),
            "#{fragment}"
        );
    }
    assert_eq!(asked(&server, "/page"), 1);
}

#[test]
fn an_element_in_a_scroll_container_is_scrolled_into_view_through_it() {
    let server = serve(|path| match path {
        "/page" => Some(html(
            "<body style=\"margin: 0\">\
             <div style=\"height: 300px\"></div>\
             <div id=box style=\"height: 50px; overflow: auto\">\
               <div style=\"height: 200px\"></div><p id=inner style=\"margin: 0\">inner</p>\
               <div style=\"height: 200px\"></div>\
             </div>\
             <div style=\"height: 900px\"></div></body>",
        )),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/page#inner"));
    tab.engine.frame(400, 100);
    let container = element_by_id(&tab.engine, "box");
    assert_eq!(tab.engine.element_scroll(container), 200.0);
    assert_eq!(tab.engine.scroll_y(), 300.0);
}

#[test]
fn a_new_document_is_scrolled_to_its_fragment_and_again_as_late_sheets_move_it() {
    let gate = Gate::new();
    // A sheet before <body> would block the page; this one comes after.
    let server = {
        let gate = gate.clone();
        serve(move |path| match path {
            "/page" => Some(html(&format!(
                "<body style=\"margin: 0\">{}<link rel=stylesheet href=late.css></body>",
                (0..40)
                    .map(|n| format!("<p id=p{n}>{n}</p>"))
                    .collect::<String>()
            ))),
            "/late.css" => {
                gate.wait();
                Some(css("p { margin: 0 0 50px 0 }"))
            }
            _ => None,
        })
    };
    let mut tab = Tab::new();
    tab.engine.navigate(server.url("/page#p20"));
    tab.wait_until(|engine| engine.navigation.is_none());
    let before = top_of(&mut tab.engine, "p20");
    assert_eq!(tab.engine.scroll_y(), before);
    gate.open();
    tab.settle();
    let after = top_of(&mut tab.engine, "p20");
    assert!(after > before);
    assert_eq!(tab.engine.scroll_y(), after, "scrolled to it again");
}

#[test]
fn a_fragment_goal_is_given_up_once_the_user_scrolls() {
    let gate = Gate::new();
    let server = {
        let gate = gate.clone();
        serve(move |path| match path {
            "/page" => Some(html(&format!(
                "<body style=\"margin: 0\">{}<link rel=stylesheet href=late.css></body>",
                (0..40)
                    .map(|n| format!("<p id=p{n}>{n}</p>"))
                    .collect::<String>()
            ))),
            "/late.css" => {
                gate.wait();
                Some(css("p { margin: 0 0 50px 0 }"))
            }
            _ => None,
        })
    };
    let mut tab = Tab::new();
    tab.engine.navigate(server.url("/page#p20"));
    tab.wait_until(|engine| engine.navigation.is_none());
    tab.engine.frame(400, 100);
    assert!(tab.engine.scroll_y() > 0.0);
    // The user scrolls while the page still loads: the fragment is
    // not scrolled to again when the late sheet moves it.
    tab.engine.set_scroll(10.0);
    gate.open();
    tab.settle();
    tab.engine.frame(400, 100);
    assert_eq!(tab.engine.scroll_y(), 10.0);
}

#[test]
fn the_same_fragment_again_replaces_the_entry_and_scrolls_again() {
    let server = serve(|path| (path == "/long").then(|| long_page("")));
    let mut tab = Tab::new();
    tab.load(server.url("/long"));
    tab.engine.navigate(server.url("/long#p30"));
    let far = top_of(&mut tab.engine, "p30");
    assert_eq!(tab.engine.scroll_y(), far);
    tab.engine.set_scroll(0.0);
    tab.engine.navigate(server.url("/long#p30"));
    tab.engine.frame(400, 100);
    assert_eq!(tab.engine.scroll_y(), far);
    assert_eq!(tab.engine.history.len(), 2);
    // Without the fragment it is another document: fetched.
    tab.load(server.url("/long"));
    assert_eq!(asked(&server, "/long"), 2);
}

#[test]
fn a_fragment_on_an_error_page_is_tried_again() {
    let server = serve(|path| (path == "/gone").then_some(Reply::Close));
    let mut tab = Tab::new();
    tab.load(server.url("/gone"));
    assert!(failed(&tab.engine));
    tab.load(server.url("/gone#part"));
    assert_eq!(asked(&server, "/gone"), 2);
}

#[test]
fn a_fragment_navigation_leaves_a_navigation_under_way_going() {
    let gate = Gate::new();
    let server = {
        let gate = gate.clone();
        serve(move |path| match path {
            "/long" => Some(long_page("")),
            "/held" => {
                gate.wait();
                Some(page("held"))
            }
            _ => None,
        })
    };
    let mut tab = Tab::new();
    tab.load(server.url("/long"));
    tab.engine.navigate(server.url("/held"));
    tab.engine.navigate(server.url("/long#p10"));
    assert!(tab.engine.navigation.is_some());
    assert_eq!(tab.engine.url(), Some(&server.url("/long#p10")));
    gate.open();
    tab.settle();
    assert_eq!(shown(&tab.engine), "held 0");
    assert_eq!(tab.engine.history.len(), 3);
    tab.engine.go_back();
    assert_eq!(tab.engine.url(), Some(&server.url("/long#p10")));
}

#[test]
fn about_silvers_own_fragment_link_navigates_within_it() {
    let mut tab = Tab::new();
    tab.engine.navigate(Url::parse("about:silver").unwrap());
    // Give the link an ID to find it by.
    tab.engine.mutate(|document| {
        let link = document
            .subtree(document.root())
            .find(|&node| document.attr(node, "href") == Some("#end"))
            .unwrap();
        document.set_attribute(link, "id", "to-end").unwrap();
    });
    let link = link_on(&mut tab.engine, "to-end");
    assert_eq!(link.url().as_str(), "about:silver#end");
    tab.engine.follow_link(&link);
    assert_eq!(tab.engine.url().map(Url::as_str), Some("about:silver#end"));
    assert_eq!(tab.engine.history.len(), 2);
    tab.engine.frame(400, 100);
    assert_eq!(
        tab.engine.scroll_y(),
        tab.engine.max_scroll(),
        "the end is at the bottom"
    );
}
