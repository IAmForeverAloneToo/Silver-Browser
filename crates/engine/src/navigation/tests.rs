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
