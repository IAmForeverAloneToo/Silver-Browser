//! The loader against a scripted server on 127.0.0.1 (docs/design/
//! net.md, "Tests"): documents and their stylesheets fetched, imported,
//! ordered, blocking, chosen and refreshed as the document changes.

use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use dom::NodeId;
use net::test_server::{Reply, TestServer};

use super::{Incoming, MAX_FETCH_THREADS};
use crate::{Engine, LoadError, LoadState, NetworkError, Url};

/// Long enough for anything a test waits for on a loaded machine.
const WAIT: Duration = Duration::from_secs(30);

/// An engine over the bundled font, and the channel its waker sends on.
struct Tab {
    engine: Engine,
    woken: Receiver<()>,
}

impl Tab {
    fn new() -> Tab {
        let mut engine = Engine::with_fonts(
            text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
                .expect("the bundled font parses"),
        );
        let (wake, woken) = channel();
        engine.set_waker(move || {
            let _ = wake.send(());
        });
        Tab { engine, woken }
    }

    /// Takes results as the waker says they come, as the shell does,
    /// until `done` holds.
    fn wait_until(&mut self, mut done: impl FnMut(&Engine) -> bool) {
        let deadline = Instant::now() + WAIT;
        loop {
            self.engine.poll_network();
            if done(&self.engine) {
                return;
            }
            let left = deadline
                .checked_duration_since(Instant::now())
                .expect("the engine gets there in time");
            self.woken
                .recv_timeout(left)
                .expect("a fetch finishes in time");
        }
    }

    /// Takes results until no fetch is in flight.
    fn settle(&mut self) {
        self.wait_until(|engine| engine.network.in_flight() == 0);
    }

    /// Loads `url` and takes results until no fetch is in flight.
    fn load(&mut self, url: Url) {
        self.engine.load(url);
        self.settle();
    }

    /// The name each stylesheet that applies goes by: its first rule's
    /// first identifier, `a` for `.a {}`.
    fn names(&self) -> Vec<String> {
        names(&self.engine)
    }
}

fn names(engine: &Engine) -> Vec<String> {
    engine
        .stylesheets()
        .iter()
        .map(|sheet| {
            sheet
                .rules
                .iter()
                .find_map(|rule| match rule {
                    css::Rule::Qualified(rule) => {
                        rule.prelude.iter().find_map(|token| match token {
                            css::Token::Ident(name) => Some(name.clone()),
                            _ => None,
                        })
                    }
                    css::Rule::At(_) => None,
                })
                .unwrap_or_default()
        })
        .collect()
}

/// A server answering each path with what `route` gives, and 404 for
/// the paths it gives nothing for.
fn serve(route: impl Fn(&str) -> Option<Reply> + Send + Sync + 'static) -> TestServer {
    TestServer::plain(move |request| {
        route(&request.target).unwrap_or_else(|| {
            Reply::with_fields(404, &[("Content-Type", "text/plain")], b"not here")
        })
    })
}

fn html(body: &str) -> Reply {
    Reply::ok("text/html", body.as_bytes())
}

fn css(body: &str) -> Reply {
    Reply::ok("text/css", body.as_bytes())
}

fn css_bytes(body: &[u8]) -> Reply {
    Reply::ok("text/css", body)
}

/// The paths the server was asked for, in the order it was.
fn paths(server: &TestServer) -> Vec<String> {
    server
        .requests()
        .into_iter()
        .map(|request| request.target)
        .collect()
}

/// The paths the server was asked for, sorted: fetches made together
/// reach it in any order.
fn sorted_paths(server: &TestServer) -> Vec<String> {
    let mut paths = paths(server);
    paths.sort_unstable();
    paths
}

/// Waits for `done`, which no fetch result tells of.
fn wait_for(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "it happens in time");
        thread::sleep(Duration::from_millis(2));
    }
}

/// A gate a server's script waits at until the test opens it.
#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    opened: Condvar,
}

impl Gate {
    fn new() -> Arc<Gate> {
        Arc::new(Gate::default())
    }

    fn wait(&self) {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        while !*open {
            open = self
                .opened
                .wait(open)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn open(&self) {
        *self.open.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.opened.notify_all();
    }
}

fn element(engine: &Engine, tag: &str) -> NodeId {
    let document = engine.document();
    document
        .subtree(document.root())
        .find(|&node| document.tag(node) == Some(tag))
        .expect("the element")
}

fn by_id(engine: &Engine, id: &str) -> NodeId {
    let document = engine.document();
    document
        .subtree(document.root())
        .find(|&node| document.attr(node, "id") == Some(id))
        .expect("the element")
}

fn color(engine: &mut Engine, id: &str) -> (f32, f32, f32) {
    let node = by_id(engine, id);
    let color = engine.styles().get(node).expect("styled").color;
    (color.r, color.g, color.b)
}

fn text(engine: &Engine) -> String {
    engine.document().text_content(engine.document().root())
}

const RED: (f32, f32, f32) = (1.0, 0.0, 0.0);
const BLACK: (f32, f32, f32) = (0.0, 0.0, 0.0);

const BODY: &str = "<body><p class=note>Some text to draw</p><div class=box></div></body>";
const SHEET_A: &str =
    ".box { width: 40px; height: 20px; background-color: #f00; border: 2px solid #00f }";
const SHEET_C: &str = ".note { color: #0a0 } .box { background-color: #ff0 }";
const SHEET_B: &str = ".box { margin-left: 10px } .note { font-size: 20px }";

#[test]
fn a_page_with_its_sheets_from_the_server_draws_what_it_draws_with_them_inline() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(&format!(
            "<!DOCTYPE html><html><head><link rel=stylesheet href=a.css>\
             <link rel=stylesheet href=sub/b.css></head>{BODY}</html>"
        ))),
        "/a.css" => Some(css(SHEET_A)),
        "/sub/b.css" => Some(css(&format!("@import 'c.css'; {SHEET_B}"))),
        "/sub/c.css" => Some(css(SHEET_C)),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/page.html"));
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    assert_eq!(tab.engine.url(), Some(&server.url("/page.html")));
    let fetched = tab.engine.frame(200, 120);

    let mut inline = Tab::new();
    inline.engine.load_document(&format!(
        "<!DOCTYPE html><html><head><style>{SHEET_A}</style><style>{SHEET_C}</style>\
         <style>{SHEET_B}</style></head>{BODY}</html>"
    ));
    assert_eq!(fetched, inline.engine.frame(200, 120));

    // And the sheets made a difference to draw.
    let mut bare = Tab::new();
    bare.engine.load_document(&format!("<html>{BODY}</html>"));
    assert_ne!(fetched, bare.engine.frame(200, 120));
    assert_eq!(
        sorted_paths(&server),
        ["/a.css", "/page.html", "/sub/b.css", "/sub/c.css"]
    );
}

#[test]
fn the_page_shown_stays_until_the_sheets_before_body_arrive() {
    let gate = Gate::new();
    let server = {
        let gate = gate.clone();
        serve(move |path| match path {
            "/page.html" => Some(html(
                "<head><style>@import 'imported.css';</style>\
                 <link rel=stylesheet href=slow.css></head><body><p>New</p></body>",
            )),
            "/slow.css" => {
                gate.wait();
                Some(css(".slow { color: #f00 }"))
            }
            "/imported.css" => Some(css(".imported {}")),
            _ => None,
        })
    };
    let mut tab = Tab::new();
    tab.engine.load(server.url("/page.html"));
    tab.wait_until(|engine| {
        matches!(&engine.incoming, Some(Incoming::Waiting { .. }))
            && engine.network.in_flight() == 1
    });
    // The document and the import are in; the blocking sheet is not, so
    // the built-in page is still the one shown.
    assert_eq!(tab.engine.load_state(), &LoadState::Loading);
    assert!(text(&tab.engine).contains("Hello from Silver Browser"));
    gate.open();
    // The first moment the new page is shown, its sheets are there.
    tab.wait_until(|engine| engine.url().is_some());
    assert!(text(&tab.engine).ends_with("New"));
    assert_eq!(tab.names(), ["imported", "", "slow"]);
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
}

#[test]
fn a_sheet_after_body_does_not_hold_the_page_back_and_restyles_it_when_it_comes() {
    let gate = Gate::new();
    let server = {
        let gate = gate.clone();
        serve(move |path| match path {
            "/page.html" => Some(html(
                "<head><link rel=stylesheet href=early.css></head>\
                 <body><p id=p>Text</p><link rel=stylesheet href=late.css></body>",
            )),
            "/early.css" => Some(css(".early {}")),
            "/late.css" => {
                gate.wait();
                Some(css(".late {} p { color: #f00 }"))
            }
            _ => None,
        })
    };
    let mut tab = Tab::new();
    tab.engine.load(server.url("/page.html"));
    tab.wait_until(|engine| engine.url().is_some());
    assert_eq!(tab.names(), ["early"]);
    assert_eq!(tab.engine.load_state(), &LoadState::Loading);
    tab.engine.frame(100, 100);
    assert_eq!(color(&mut tab.engine, "p"), BLACK);
    let runs = tab.engine.style_runs();

    gate.open();
    tab.settle();
    assert_eq!(tab.names(), ["early", "late"]);
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    assert!(tab.engine.needs_frame());
    tab.engine.frame(100, 100);
    assert_eq!(tab.engine.style_runs(), runs + 1, "a whole cascade");
    assert_eq!(color(&mut tab.engine, "p"), RED);
}

#[test]
fn sheets_take_the_tree_order_of_their_elements_not_the_order_they_arrive_in() {
    let gate = Gate::new();
    let server = {
        let gate = gate.clone();
        serve(move |path| match path {
            "/page.html" => Some(html(
                "<body><p id=p>Text</p><link rel=stylesheet href=first.css>\
                 <style>.middle {}</style><link rel=stylesheet href=second.css></body>",
            )),
            "/first.css" => {
                gate.wait();
                Some(css(".first {} p { color: #f00 }"))
            }
            "/second.css" => Some(css(".second {} p { color: #00f }")),
            _ => None,
        })
    };
    let mut tab = Tab::new();
    tab.engine.load(server.url("/page.html"));
    tab.wait_until(|engine| names(engine) == ["middle", "second"]);
    gate.open();
    tab.settle();
    assert_eq!(tab.names(), ["first", "middle", "second"]);
    // The later sheet in tree order wins, though it arrived first.
    assert_eq!(color(&mut tab.engine, "p"), (0.0, 0.0, 1.0));
}

#[test]
fn imports_resolve_against_their_sheet_and_skip_cycles() {
    let server = serve(|path| match path {
        "/page.html" => Some(html("<link rel=stylesheet href=css/a.css>")),
        "/css/a.css" => Some(css("@import 'b.css'; .a {}")),
        // b imports a, which is above it, and c.
        "/css/b.css" => Some(css("@import url(a.css); @import '/css/c.css#x'; .b {}")),
        // c imports b, above it, and d, which is missing.
        "/css/c.css" => Some(css("@import 'b.css'; @import 'd.css'; .c {}")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/page.html"));
    assert_eq!(tab.names(), ["c", "b", "a"]);
    assert_eq!(
        paths(&server),
        [
            "/page.html",
            "/css/a.css",
            "/css/b.css",
            "/css/c.css",
            "/css/d.css"
        ]
    );
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
}

#[test]
fn a_redirected_sheets_imports_resolve_against_the_url_it_was_requested_from() {
    let server = serve(|path| match path {
        "/page.html" => Some(html("<link rel=stylesheet href=a.css>")),
        "/a.css" => Some(Reply::with_fields(
            302,
            &[("Location", "/moved/a.css")],
            b"",
        )),
        "/moved/a.css" => Some(css("@import 'b.css'; .a {}")),
        "/b.css" => Some(css(".b {}")),
        "/moved/b.css" => Some(css(".wrong {}")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/page.html"));
    assert_eq!(tab.names(), ["b", "a"]);
}

#[test]
fn imports_go_sixteen_deep() {
    let server = serve(|path| {
        let depth: usize = path
            .strip_prefix("/d")?
            .strip_suffix(".css")?
            .parse()
            .ok()?;
        Some(css(&format!(
            "@import 'd{}.css'; .d{depth} {{}}",
            depth + 1
        )))
    });
    let page = TestServer::plain({
        let sheet = server.url("/d0.css");
        move |_| html(&format!("<link rel=stylesheet href='{}'>", sheet.as_str()))
    });
    let mut tab = Tab::new();
    tab.load(page.url("/"));
    let expected: Vec<String> = (0..=16).rev().map(|depth| format!("d{depth}")).collect();
    assert_eq!(tab.names(), expected);
    assert_eq!(paths(&server).len(), 17);
}

#[test]
fn a_style_elements_imports_resolve_against_the_base_url() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<head><base href=/dir/><style>@import 'x.css' screen; @import 'y.css' print;\
             .s {}</style></head>",
        )),
        "/dir/x.css" => Some(css(".x {}")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/page.html"));
    assert_eq!(tab.names(), ["x", "s"]);
    assert_eq!(paths(&server), ["/page.html", "/dir/x.css"]);
}

#[test]
fn a_link_resolves_against_the_base_url_as_it_was_where_the_parser_put_it() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<head><link rel=stylesheet href=early.css><base href=/dir/>\
             <base href=/other/><link rel=stylesheet href=late.css></head>",
        )),
        "/data.html" => Some(html(
            "<head><base href='data:text/css,.x{}'><link rel=stylesheet href=a.css></head>",
        )),
        "/early.css" => Some(css(".early {}")),
        "/dir/late.css" => Some(css(".late {}")),
        "/a.css" => Some(css(".a {}")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/page.html"));
    assert_eq!(tab.names(), ["early", "late"]);
    // A data: base leaves the document's URL in place.
    tab.load(server.url("/data.html"));
    assert_eq!(tab.names(), ["a"]);
}

#[test]
fn the_first_titled_sheet_names_the_preferred_set() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<head><link rel=stylesheet href=plain.css>\
             <link rel=stylesheet href=missing.css title=Gone>\
             <link rel=stylesheet href=one.css title=One>\
             <style title=Two>.two {}</style><style title=One>.one2 {}</style>\
             <link rel='alternate stylesheet' href=alt.css title=Two></head>",
        )),
        "/plain.css" => Some(css(".plain {}")),
        "/one.css" => Some(css(".one {}")),
        "/alt.css" => Some(css(".alt {}")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/page.html"));
    // The sheet titled Gone failed, so One, the next, names the set.
    assert_eq!(tab.names(), ["plain", "one", "one2"]);
    assert!(!paths(&server).contains(&"/alt.css".to_owned()));
}

#[test]
fn only_a_stylesheet_of_the_right_type_applies() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<head><link rel=stylesheet href=plain.txt><link rel=stylesheet href=none.css>\
             <link rel=stylesheet href=nosniff.css><link rel=stylesheet href=charset.css>\
             <link rel=stylesheet href=missing.css><link rel=stylesheet href=closed.css>\
             <link rel=stylesheet href=ok.css></head>",
        )),
        "/plain.txt" => Some(Reply::ok("text/plain", b".plain {}")),
        "/none.css" => Some(Reply::with_fields(200, &[], b".none {}")),
        "/nosniff.css" => Some(Reply::with_fields(
            200,
            &[("X-Content-Type-Options", "nosniff")],
            b".nosniff {}",
        )),
        "/charset.css" => Some(Reply::ok("text/css; charset=utf-8", b".charset {}")),
        "/missing.css" => Some(Reply::with_fields(
            404,
            &[("Content-Type", "text/css")],
            b".missing {}",
        )),
        "/closed.css" => Some(Reply::Close),
        "/ok.css" => Some(css(".ok {}")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/page.html"));
    assert_eq!(tab.names(), ["none", "charset", "ok"]);
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
}

#[test]
fn only_a_link_to_a_stylesheet_for_the_screen_is_fetched() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<head><link rel=stylesheet type=text/plain href=a.css>\
             <link rel=stylesheet type='text/css; charset=utf-8' href=b.css>\
             <link rel=stylesheet type='' href=c.css>\
             <link rel=stylesheet media=print href=d.css>\
             <link rel=stylesheet media='screen, print' href=e.css>\
             <link rel='alternate stylesheet' href=f.css>\
             <link rel='  STYLESHEET  ' href=g.css>\
             <link rel=stylesheet disabled href=h.css>\
             <link rel=stylesheet href=''>\
             <link rel=icon href=i.css>\
             <link rel=stylesheet href='http://[::1'>\
             <style type=text/plain>.j {}</style><style media=print>.k {}</style>\
             <style type=TEXT/CSS media=all>.l {}</style></head>",
        )),
        path => path
            .strip_prefix('/')?
            .strip_suffix(".css")
            .map(|name| css(&format!(".{name} {{}}"))),
    });
    let mut tab = Tab::new();
    tab.load(server.url("/page.html"));
    assert_eq!(tab.names(), ["b", "c", "e", "g", "l"]);
    assert_eq!(
        sorted_paths(&server),
        ["/b.css", "/c.css", "/e.css", "/g.css", "/page.html"]
    );
}

#[test]
fn a_sheet_follows_its_element_through_the_documents_changes() {
    let server = serve(|path| {
        if path == "/page.html" {
            return Some(html(
                "<head><link rel=stylesheet href=a.css></head><body><p>x</p></body>",
            ));
        }
        let name = path.rsplit('/').next()?.strip_suffix(".css")?;
        Some(css(&format!(".{name} {{}}")))
    });
    let mut tab = Tab::new();
    tab.load(server.url("/page.html"));
    assert_eq!(tab.names(), ["a"]);
    let head = element(&tab.engine, "head");
    let a = element(&tab.engine, "link");
    let insert_link = |tab: &mut Tab, href: &str| {
        tab.engine.mutate(|document| {
            let link = document.create_element("link").unwrap();
            document.set_attribute(link, "rel", "stylesheet").unwrap();
            document.set_attribute(link, "href", href).unwrap();
            document.append_child(head, link).unwrap();
            link
        })
    };

    // Inserted: fetched and applied.
    let b = insert_link(&mut tab, "b.css");
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    tab.settle();
    assert_eq!(tab.names(), ["a", "b"]);

    // Its href changed: fetched again.
    tab.engine.mutate(|document| {
        document.set_attribute(a, "href", "c.css").unwrap();
    });
    tab.settle();
    assert_eq!(tab.names(), ["c", "b"]);

    // Moved before the other: the sheets follow tree order, and nothing
    // is fetched again.
    let requests = paths(&server).len();
    tab.engine.mutate(|document| {
        document.remove_child(head, b).unwrap();
        document.insert_before(head, b, Some(a)).unwrap();
    });
    assert_eq!(tab.names(), ["b", "c"]);
    assert_eq!(tab.engine.network.in_flight(), 0);
    assert_eq!(paths(&server).len(), requests);

    // For another medium, and then removed: gone.
    tab.engine.mutate(|document| {
        document.set_attribute(b, "media", "print").unwrap();
    });
    assert_eq!(tab.names(), ["c"]);
    tab.engine.mutate(|document| {
        document.remove_child(head, a).unwrap();
    });
    assert!(tab.names().is_empty());

    // A <style> inserted, and its text changed.
    let style_text = tab.engine.mutate(|document| {
        let style = document.create_element("style").unwrap();
        let text = document.create_text(".s {}").unwrap();
        document.append_child(style, text).unwrap();
        document.append_child(head, style).unwrap();
        text
    });
    assert_eq!(tab.names(), ["s"]);
    tab.engine.mutate(|document| {
        document.set_text(style_text, ".t {}").unwrap();
    });
    assert_eq!(tab.names(), ["t"]);

    // A <base> inserted leaves the sheets fetched before it alone, and
    // holds for those fetched after.
    insert_link(&mut tab, "f.css");
    tab.settle();
    assert_eq!(tab.names(), ["t", "f"]);
    let requests = paths(&server).len();
    tab.engine.mutate(|document| {
        let base = document.create_element("base").unwrap();
        document.set_attribute(base, "href", "/dir/").unwrap();
        document.insert_before(head, base, Some(b)).unwrap();
    });
    assert_eq!(tab.engine.network.in_flight(), 0);
    assert_eq!(paths(&server).len(), requests);
    assert_eq!(tab.names(), ["t", "f"]);
    tab.engine.mutate(|document| {
        document.set_attribute(b, "media", "all").unwrap();
    });
    tab.settle();
    insert_link(&mut tab, "e.css");
    tab.settle();
    assert_eq!(paths(&server)[requests..], ["/dir/b.css", "/dir/e.css"]);
    assert_eq!(tab.names(), ["b", "t", "f", "e"]);

    // Titles given and taken away choose the preferred set afresh.
    let e = last(&tab.engine, "link");
    tab.engine.mutate(|document| {
        document.set_attribute(e, "title", "One").unwrap();
        document.set_attribute(b, "title", "Two").unwrap();
    });
    assert_eq!(tab.names(), ["b", "t", "f"]);
    tab.engine.mutate(|document| {
        document.remove_attribute(b, "title").unwrap();
    });
    assert_eq!(tab.names(), ["b", "t", "f", "e"]);
    assert_eq!(tab.engine.network.in_flight(), 0);
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
}

/// The last `tag` element in tree order.
fn last(engine: &Engine, tag: &str) -> NodeId {
    let document = engine.document();
    document
        .subtree(document.root())
        .filter(|&node| document.tag(node) == Some(tag))
        .last()
        .expect("the element")
}

#[test]
fn a_link_removed_before_its_sheet_arrives_is_aborted() {
    let gate = Gate::new();
    let server = {
        let gate = gate.clone();
        serve(move |path| match path {
            "/page.html" => Some(html(
                "<body><p>x</p><link rel=stylesheet href=slow.css></body>",
            )),
            "/slow.css" => {
                gate.wait();
                Some(css(".slow {}"))
            }
            _ => None,
        })
    };
    let mut tab = Tab::new();
    tab.engine.load(server.url("/page.html"));
    tab.wait_until(|engine| engine.url().is_some());
    wait_for(|| paths(&server).contains(&"/slow.css".to_owned()));
    assert_eq!(tab.engine.load_state(), &LoadState::Loading);
    let link = element(&tab.engine, "link");
    let body = element(&tab.engine, "body");
    tab.engine.mutate(|document| {
        document.remove_child(body, link).unwrap();
    });
    // Nothing is left to wait for.
    assert_eq!(tab.engine.network.in_flight(), 0);
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    // The abort ends the fetch's wait without the server's answer.
    wait_for(|| tab.engine.network.pool().running() == 0);
    gate.open();
    tab.engine.poll_network();
    assert!(tab.names().is_empty());
}

#[test]
fn a_document_that_is_not_html_fails_as_unsupported_and_the_page_stays() {
    let server = serve(|path| match path {
        "/text" => Some(Reply::ok("text/plain", b"just text")),
        "/missing.html" => Some(Reply::with_fields(
            404,
            &[("Content-Type", "text/html")],
            b"<p>Not here</p>",
        )),
        "/closed" => Some(Reply::Close),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/text"));
    assert_eq!(
        tab.engine.load_state(),
        &LoadState::Failed(LoadError::Unsupported("text/plain".into()))
    );
    assert_eq!(tab.engine.url(), None, "the built-in page stays");

    // An error status is still a document.
    tab.load(server.url("/missing.html"));
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    assert_eq!(text(&tab.engine), "Not here");

    // A network error leaves that document in place.
    tab.load(server.url("/closed"));
    assert!(matches!(
        tab.engine.load_state(),
        LoadState::Failed(LoadError::Network(_))
    ));
    assert_eq!(tab.engine.url(), Some(&server.url("/missing.html")));
}

#[test]
fn a_redirected_document_is_at_its_last_url() {
    let server = serve(|path| match path {
        "/old" => Some(Reply::with_fields(
            301,
            &[("Location", "/new/page.html")],
            b"",
        )),
        "/new/page.html" => Some(html("<link rel=stylesheet href=a.css>")),
        "/new/a.css" => Some(css(".a {}")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/old"));
    assert_eq!(tab.engine.url(), Some(&server.url("/new/page.html")));
    assert_eq!(tab.names(), ["a"]);
}

#[test]
fn a_page_no_server_answered_is_public_and_may_not_reach_loopback() {
    let server = serve(|path| (path == "/a.css").then(|| css(".a {}")));
    let page = format!(
        "data:text/html,<link rel=stylesheet href={}><p>x",
        server.url("/a.css").as_str()
    );
    let mut tab = Tab::new();
    tab.load(Url::parse(&page).unwrap());
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    assert_eq!(text(&tab.engine), "x");
    assert!(tab.names().is_empty());
    assert!(paths(&server).is_empty());
}

#[test]
fn a_document_and_its_sheets_are_decoded_in_their_encodings() {
    let server = serve(|path| match path {
        // The sheet takes the document's encoding.
        "/legacy.html" => Some(Reply::ok(
            "text/html; charset=windows-1252",
            b"<link rel=stylesheet href=a.css><p id=one class=caf\xe9>caf\xe9</p>",
        )),
        "/a.css" => Some(css_bytes(b"#one.caf\xe9 { color: #f00 }")),
        // Here the link's charset says, and the import takes its
        // importer's; with neither, the bytes are not UTF-8.
        "/utf8.html" => Some(Reply::ok(
            "text/html; charset=utf-8",
            "<link rel=stylesheet href=b.css charset=windows-1252>\
             <link rel=stylesheet href=c.css><link rel=stylesheet href=e.css>\
             <link rel=stylesheet href=f.css>\
             <p id=two class=café>x</p><p id=three class=café>x</p>\
             <p id=four class=café>x</p><p id=five class=café>x</p>"
                .as_bytes(),
        )),
        "/b.css" => Some(css_bytes(b"#two.caf\xe9 { color: #f00 }")),
        "/c.css" => Some(css_bytes(b"@charset \"windows-1252\"; @import 'd.css';")),
        "/d.css" => Some(css_bytes(b"#three.caf\xe9 { color: #f00 }")),
        "/e.css" => Some(css_bytes(b"#four.caf\xe9 { color: #f00 }")),
        // The sheet's own Content-Type says, over the document's.
        "/f.css" => Some(Reply::ok(
            "text/css; charset=windows-1252",
            b"#five.caf\xe9 { color: #f00 }",
        )),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/legacy.html"));
    assert_eq!(text(&tab.engine), "café");
    assert_eq!(color(&mut tab.engine, "one"), RED);
    tab.load(server.url("/utf8.html"));
    assert_eq!(color(&mut tab.engine, "two"), RED);
    assert_eq!(color(&mut tab.engine, "three"), RED);
    assert_eq!(color(&mut tab.engine, "four"), BLACK);
    assert_eq!(color(&mut tab.engine, "five"), RED);
}

#[test]
fn a_new_load_aborts_the_one_under_way() {
    let gate = Gate::new();
    let server = {
        let gate = gate.clone();
        serve(move |path| match path {
            "/first.html" => {
                gate.wait();
                Some(html("<p>First</p>"))
            }
            "/second.html" => Some(html("<p>Second</p>")),
            _ => None,
        })
    };
    let mut tab = Tab::new();
    tab.engine.load(server.url("/first.html"));
    wait_for(|| paths(&server).contains(&"/first.html".to_owned()));
    tab.load(server.url("/second.html"));
    assert_eq!(tab.engine.url(), Some(&server.url("/second.html")));
    wait_for(|| tab.engine.network.pool().running() == 0);
    gate.open();
    tab.engine.poll_network();
    assert_eq!(text(&tab.engine), "Second");
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
}

#[test]
fn a_new_page_aborts_the_fetches_of_the_page_it_replaces() {
    let gate = Gate::new();
    let server = {
        let gate = gate.clone();
        serve(move |path| match path {
            "/first.html" => Some(html(
                "<body><p>First</p><link rel=stylesheet href=slow.css></body>",
            )),
            "/slow.css" => {
                gate.wait();
                Some(css(".slow {}"))
            }
            "/second.html" => Some(html("<p>Second</p>")),
            _ => None,
        })
    };
    let mut tab = Tab::new();
    tab.engine.load(server.url("/first.html"));
    tab.wait_until(|engine| engine.url().is_some());
    wait_for(|| paths(&server).contains(&"/slow.css".to_owned()));
    tab.load(server.url("/second.html"));
    assert_eq!(text(&tab.engine), "Second");
    assert_eq!(tab.engine.network.in_flight(), 0);
    wait_for(|| tab.engine.network.pool().running() == 0);
    gate.open();
}

#[test]
fn one_poll_takes_every_result_that_has_arrived() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<body><p>x</p><link rel=stylesheet href=a.css><link rel=stylesheet href=b.css>\
             </body>",
        )),
        "/a.css" => Some(css(".a {}")),
        "/b.css" => Some(css(".b {}")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.load(server.url("/page.html"));
    tab.wait_until(|engine| engine.url().is_some());
    wait_for(|| paths(&server).len() == 3 && tab.engine.network.pool().running() == 0);
    assert!(tab.engine.poll_network(), "a frame is wanted");
    assert_eq!(tab.names(), ["a", "b"]);
}

#[test]
fn an_engine_dropped_aborts_its_fetches() {
    let server = TestServer::plain(|_| Reply::Hang);
    let mut tab = Tab::new();
    tab.engine.load(server.url("/"));
    wait_for(|| server.requests().len() == 1);
    let running = tab.engine.network.pool().running_probe();
    assert_eq!(running(), 1);
    drop(tab);
    wait_for(|| running() == 0);
}

#[test]
fn no_more_than_thirty_two_fetches_run_at_once_and_the_sheets_keep_their_order() {
    const SHEETS: usize = 40;
    let gate = Gate::new();
    let server = {
        let gate = gate.clone();
        serve(move |path| {
            if path == "/page.html" {
                let links: String = (0..SHEETS)
                    .map(|i| format!("<link rel=stylesheet href=s{i}.css>"))
                    .collect();
                return Some(html(&format!("<head>{links}</head>")));
            }
            let name = path.strip_prefix('/')?.strip_suffix(".css")?.to_owned();
            gate.wait();
            Some(css(&format!(".{name} {{}}")))
        })
    };
    let mut tab = Tab::new();
    tab.engine.load(server.url("/page.html"));
    tab.wait_until(|engine| matches!(&engine.incoming, Some(Incoming::Waiting { .. })));
    wait_for(|| tab.engine.network.pool().running() == MAX_FETCH_THREADS);
    thread::sleep(Duration::from_millis(100));
    assert_eq!(tab.engine.network.pool().most_running(), MAX_FETCH_THREADS);
    gate.open();
    tab.settle();
    let expected: Vec<String> = (0..SHEETS).map(|i| format!("s{i}")).collect();
    assert_eq!(tab.names(), expected);
}

#[test]
fn a_document_given_to_the_engine_fetches_nothing() {
    let server = serve(|path| (path == "/a.css").then(|| css(".a {}")));
    let mut tab = Tab::new();
    tab.engine.load_document(&format!(
        "<link rel=stylesheet href={}><style>@import '{}'; .s {{}}</style>",
        server.url("/a.css").as_str(),
        server.url("/a.css").as_str()
    ));
    assert_eq!(tab.names(), ["s"]);
    assert_eq!(tab.engine.network.in_flight(), 0);
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    assert_eq!(tab.engine.url(), None);
    assert!(paths(&server).is_empty());
}

#[test]
fn a_document_given_while_a_load_is_under_way_replaces_it() {
    let gate = Gate::new();
    let server = {
        let gate = gate.clone();
        serve(move |path| {
            (path == "/page.html").then(|| {
                gate.wait();
                html("<p>Late</p>")
            })
        })
    };
    let mut tab = Tab::new();
    tab.engine.load(server.url("/page.html"));
    tab.engine.load_document("<p>Given</p>");
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    assert_eq!(tab.engine.network.in_flight(), 0);
    wait_for(|| tab.engine.network.pool().running() == 0);
    gate.open();
    tab.engine.poll_network();
    assert_eq!(text(&tab.engine), "Given");
}

#[test]
fn an_aborted_fetch_ends_its_thread_and_its_result_is_not_taken() {
    let server = TestServer::plain(|_| Reply::Hang);
    let mut network = super::Network::new();
    let request = net::Request::get(server.url("/"), net::Destination::Document);
    network.fetch(0, super::Target::Document, request);
    wait_for(|| server.requests().len() == 1);
    assert_eq!(network.pool().running(), 1);
    network.cancel_all();
    wait_for(|| network.pool().running() == 0);
    assert!(network.take().is_none());
}

#[test]
fn every_load_error_says_something() {
    let said: Vec<String> = [
        LoadError::Network(NetworkError::Timeout),
        LoadError::Unsupported("text/plain".into()),
        LoadError::Lost,
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    assert_eq!(
        said,
        [
            "timed out",
            "text/plain is not shown",
            "the fetch's thread failed"
        ]
    );
}
