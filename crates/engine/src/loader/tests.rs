//! The loader against a scripted server on 127.0.0.1 (docs/design/
//! net.md, "Tests"): documents and their stylesheets fetched, imported,
//! ordered, blocking, chosen and refreshed as the document changes.

use std::thread;
use std::time::Duration;

use dom::NodeId;
use net::test_server::{Reply, TestServer};

use super::{Incoming, MAX_FETCH_THREADS};
use crate::testing::*;
use crate::{Engine, LoadError, LoadState, NetworkError, Url};

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
    tab.engine.navigate(server.url("/page.html"));
    tab.wait_until(|engine| {
        matches!(incoming(engine), Some(Incoming::Parsing { .. }))
            && engine.network.in_flight() == 1
    });
    // The document and the import are in; the blocking sheet is not, so
    // the initial about:blank is still the one shown.
    assert_eq!(tab.engine.load_state(), &LoadState::Loading);
    assert_eq!(tab.engine.url().map(Url::as_str), Some("about:blank"));
    assert_eq!(text(&tab.engine), "");
    gate.open();
    // The first moment the new page is shown, its sheets are there.
    tab.wait_until(|engine| engine.navigation.is_none());
    assert!(text(&tab.engine).ends_with("New"));
    assert_eq!(tab.names(), ["imported", "slow"]);
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
    tab.engine.navigate(server.url("/page.html"));
    tab.wait_until(|engine| engine.navigation.is_none());
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
    tab.engine.navigate(server.url("/page.html"));
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
    // The import for print is fetched too, for a print.
    assert_eq!(
        sorted_paths(&server),
        ["/dir/x.css", "/dir/y.css", "/page.html"]
    );
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
fn every_link_to_a_stylesheet_is_fetched_and_its_media_says_where_it_applies() {
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
    // The sheets for print, `d` and `k`, are made, and apply on paper.
    assert_eq!(tab.names(), ["b", "c", "e", "g", "l"]);
    assert_eq!(
        sorted_paths(&server),
        [
            "/b.css",
            "/c.css",
            "/d.css",
            "/e.css",
            "/g.css",
            "/page.html"
        ]
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

    // For another medium: out of force, and not fetched again. Then
    // removed: gone.
    tab.engine.mutate(|document| {
        document.set_attribute(b, "media", "print").unwrap();
    });
    assert_eq!(tab.names(), ["c"]);
    assert_eq!(tab.engine.network.in_flight(), 0);
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
    // Its media changed back, `b` applies again as it was fetched.
    tab.engine.mutate(|document| {
        document.set_attribute(b, "media", "all").unwrap();
    });
    tab.settle();
    insert_link(&mut tab, "e.css");
    tab.settle();
    assert_eq!(paths(&server)[requests..], ["/dir/e.css"]);
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

#[test]
fn a_sheet_for_another_medium_does_not_hold_the_page_back() {
    let gate = Gate::new();
    let server = {
        let gate = gate.clone();
        serve(move |path| match path {
            "/page.html" => Some(html(
                "<head><link rel=stylesheet media=print href=paper.css>\
                 <link rel=stylesheet media='(min-width: 100000px)' href=wide.css>\
                 <link rel=stylesheet media='screen' href=screen.css></head>\
                 <body><p id=p>New</p></body>",
            )),
            "/paper.css" | "/wide.css" => {
                gate.wait();
                Some(css(".late { } p { color: #f00 }"))
            }
            "/screen.css" => Some(css(".screen {}")),
            _ => None,
        })
    };
    let mut tab = Tab::new();
    tab.engine.navigate(server.url("/page.html"));
    // Shown as soon as the sheet for the screen is in.
    tab.wait_until(|engine| engine.navigation.is_none());
    assert!(text(&tab.engine).ends_with("New"));
    assert_eq!(tab.names(), ["screen"]);
    assert_eq!(tab.engine.load_state(), &LoadState::Loading);
    gate.open();
    tab.settle();
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    tab.engine.frame(100, 100);
    assert_eq!(color(&mut tab.engine, "p"), BLACK, "neither applies here");
}

#[test]
fn imports_carry_their_layers_and_conditions() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<head><link rel=stylesheet href=main.css></head><body><p id=p>Text</p></body>",
        )),
        // `top` is named first though its sheet is missing, so `base`,
        // after it, wins over the block that fills it.
        "/main.css" => Some(css(
            "@layer first; @import 'missing.css' layer(top); @import url(base.css) layer(base);\
             @import 'narrow.css' supports(display: grid) (max-width: 300px);\
             @import 'never.css' supports(display: nonsense);\
             @import 'bad.css' layer(initial); @import 'bad.css' supports(nonsense);\
             @layer top { p { color: #00f } }",
        )),
        "/base.css" => Some(css(".base {} p { color: #f00 }")),
        "/narrow.css" => Some(css(".narrow {} p { color: #0f0 !important }")),
        "/never.css" => Some(css(".never {} p { color: #000 !important }")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/page.html"));
    // The invalid imports are not fetched; the others are, whatever
    // their conditions.
    assert_eq!(
        sorted_paths(&server),
        [
            "/base.css",
            "/main.css",
            "/missing.css",
            "/narrow.css",
            "/never.css",
            "/page.html"
        ]
    );
    tab.engine.frame(400, 100);
    assert_eq!(color(&mut tab.engine, "p"), RED);
    assert_eq!(tab.names(), ["base"]);
    // Narrower than 300px, the import for it applies.
    tab.engine.frame(200, 100);
    assert_eq!(color(&mut tab.engine, "p"), (0.0, 1.0, 0.0));
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
    tab.engine.navigate(server.url("/page.html"));
    tab.wait_until(|engine| engine.navigation.is_none());
    wait_for(|| paths(&server).contains(&"/slow.css".to_owned()));
    assert_eq!(tab.engine.load_state(), &LoadState::Loading);
    let link = element(&tab.engine, "link");
    let body = element(&tab.engine, "body");
    tab.engine.mutate(|document| {
        document.remove_child(body, link).unwrap();
    });
    // Nothing is left to wait for.
    let sheets_in_flight = tab
        .engine
        .network
        .flights
        .values()
        .filter(|flight| matches!(flight.target, super::Target::Sheet { .. }))
        .count();
    assert_eq!(sheets_in_flight, 0);
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    // The abort ends the fetch's wait without the server's answer.
    wait_for(|| tab.engine.network.pool().running() == 0);
    gate.open();
    tab.engine.poll_network();
    assert!(tab.names().is_empty());
}

#[test]
fn the_content_language_of_the_response_is_the_documents_last_resort() {
    // `:lang()` falls back on the response's language when no `lang`
    // attribute or `meta` pragma gives one (M16), if it names one.
    const PAGE: &str = "<style>p:lang(fr) { color: #f00 } p:lang('') { color: #0f0 }</style>\
                        <p id=a>x</p>";
    let server = serve(|path| {
        let languages: &[(&str, &str)] = match path {
            "/fr.html" => &[("Content-Language", "fr-CA")],
            "/spaced.html" => &[("Content-Language", " , fr ,")],
            "/two.html" => &[("Content-Language", "fr, en")],
            "/twice.html" => &[("Content-Language", "fr"), ("Content-Language", "en")],
            "/none.html" => &[],
            _ => return None,
        };
        let mut fields = vec![("Content-Type", "text/html")];
        fields.extend_from_slice(languages);
        Some(Reply::with_fields(200, &fields, PAGE.as_bytes()))
    });
    let red = style::computed::Color::rgb8(255, 0, 0);
    let green = style::computed::Color::rgb8(0, 255, 0);
    for (path, expected) in [
        ("/fr.html", red),
        ("/spaced.html", red),
        ("/two.html", green),
        ("/twice.html", green),
        ("/none.html", green),
    ] {
        let mut tab = Tab::new();
        tab.load(server.url(path));
        let document = tab.engine.document();
        let p = document
            .subtree(document.root())
            .find(|&node| document.attr(node, "id") == Some("a"))
            .expect("the paragraph");
        let color = tab.engine.styles().get(p).expect("styled").color;
        assert_eq!(color, expected, "{path}");
        // A restyle reads it too: a `style` attribute restyles its
        // element.
        let before = tab.engine.restyled_elements();
        tab.engine.mutate(|document| {
            let _ = document.set_attribute(p, "style", "padding: 1px");
        });
        let color = tab.engine.styles().get(p).expect("styled").color;
        assert_eq!(color, expected, "{path}, restyled");
        assert!(
            tab.engine.restyled_elements() > before,
            "{path}: restyled, not cascaded"
        );
    }
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
fn a_document_in_an_encoding_not_decoded_yet_says_which() {
    let server = serve(|path| match path {
        "/sjis.html" => Some(Reply::ok(
            "text/html; charset=shift_jis",
            b"<p>ASCII stays, \x82\xa0 does not</p>",
        )),
        "/utf8.html" => Some(html("<p>x</p>")),
        _ => None,
    });
    let mut tab = Tab::new();
    assert_eq!(tab.engine.undecodable_encoding(), None);
    tab.load(server.url("/sjis.html"));
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    assert_eq!(tab.engine.undecodable_encoding(), Some("Shift_JIS"));
    assert_eq!(text(&tab.engine), "ASCII stays, \u{fffd}\u{fffd} does not");
    tab.load(server.url("/utf8.html"));
    assert_eq!(tab.engine.undecodable_encoding(), None);
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
    tab.engine.navigate(server.url("/first.html"));
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
    tab.engine.navigate(server.url("/first.html"));
    tab.wait_until(|engine| engine.navigation.is_none());
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
    tab.engine.navigate(server.url("/page.html"));
    tab.wait_until(|engine| engine.navigation.is_none());
    wait_for(|| paths(&server).len() == 3 && tab.engine.network.pool().running() == 0);
    assert!(tab.engine.poll_network(), "a frame is wanted");
    assert_eq!(tab.names(), ["a", "b"]);
}

#[test]
fn an_engine_dropped_aborts_its_fetches() {
    let server = TestServer::plain(|_| Reply::Hang);
    let mut tab = Tab::new();
    tab.engine.navigate(server.url("/"));
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
    tab.engine.navigate(server.url("/page.html"));
    tab.wait_until(|engine| matches!(incoming(engine), Some(Incoming::Parsing { .. })));
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
    tab.engine.navigate(server.url("/page.html"));
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
    let said: Vec<String> = [LoadError::Network(NetworkError::Timeout), LoadError::Lost]
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(said, ["timed out", "the fetch's thread failed"]);
}

// Images (M23.1, M23.5): fetched once, read off the engine's thread,
// waited for by the load, and decoded when first asked for.

fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&vec![200; (width * height * 4) as usize])
        .unwrap();
    writer.finish().unwrap();
    out
}

fn image_state(engine: &Engine, url: &Url) -> Option<crate::image_store::ImageState> {
    engine
        .shown
        .images
        .get(url.as_str())
        .map(|entry| entry.state.clone())
}

/// The number of the image at `url` in the shown document.
fn image_id(engine: &Engine, url: &Url) -> u64 {
    engine
        .shown
        .images
        .get(url.as_str())
        .expect("an image of the document")
        .id
}

/// Navigates to `url` and takes results until its page is shown and
/// loaded, images and all.
pub(super) fn load_all(tab: &mut Tab, url: Url) {
    tab.engine.navigate(url.clone());
    tab.wait_until(|engine| {
        incoming(engine).is_none()
            && engine.url() == Some(&url)
            && engine.load_state() == &LoadState::Loaded
    });
}

/// A server whose `/page.html` is `first`, then after `wait` the rest:
/// HTML, its length said up front.
fn slow_page(first: &'static str, rest: &'static str, wait: Duration) -> TestServer {
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{first}",
        first.len() + rest.len()
    );
    serve(move |path| match path {
        "/page.html" => Some(Reply::Pieces(vec![
            (Duration::ZERO, head.clone().into_bytes()),
            (wait, rest.as_bytes().to_vec()),
        ])),
        "/other.html" => Some(html("<p>other</p>")),
        _ => None,
    })
}

/// A page's first piece: past the resource header, so that it is known
/// to be HTML at once, with a body already.
pub(super) const FIRST: &str = concat!(
    "<!DOCTYPE html><title>Slow</title><body><p>first</p><!--",
    "................................................................................",
    "................................................................................",
    "................................................................................",
    "................................................................................",
    "................................................................................",
    "................................................................................",
    "................................................................................",
    "................................................................................",
    "................................................................................",
    "................................................................................",
    "................................................................................",
    "................................................................................",
    "................................................................................",
    "................................................................................",
    "................................................................................",
    "................................................................................",
    "................................................................................",
    "................................................................................",
    "-->"
);

#[test]
fn a_document_is_shown_before_its_body_has_all_arrived_and_grows() {
    let server = slow_page(FIRST, "<p>after</p>", Duration::from_millis(1500));
    let page = server.url("/page.html");
    let mut tab = Tab::new();
    tab.engine.navigate(page.clone());
    // Shown with what has come: it has a body and no sheet holds it.
    tab.wait_until(|engine| engine.navigation.is_none());
    assert_eq!(tab.engine.url(), Some(&page));
    assert_eq!(text(&tab.engine), "Slowfirst");
    assert_eq!(tab.engine.title(), "Slow");
    assert_eq!(tab.engine.load_state(), &LoadState::Loading);
    let frame = tab.engine.frame(200, 100);
    // The rest comes, and the document grows: a change like any other.
    load_all(&mut tab, page);
    assert_eq!(text(&tab.engine), "Slowfirstafter");
    assert!(tab.engine.needs_frame() || tab.engine.frame(200, 100) != frame);
}

#[test]
fn a_late_meta_starts_a_streamed_document_again_in_its_encoding() {
    // Past the prescan, a meta naming windows-1252, arriving after a
    // wait: the first piece is parsed as UTF-8, then all again.
    let server = {
        let first = FIRST.as_bytes().to_vec();
        let rest = b"<meta charset=windows-1252><p>caf\xE9".to_vec();
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n",
            first.len() + rest.len()
        );
        serve(move |path| match path {
            "/page.html" => {
                let mut start = head.clone().into_bytes();
                start.extend(&first);
                Some(Reply::Pieces(vec![
                    (Duration::ZERO, start),
                    (Duration::from_millis(300), rest.clone()),
                ]))
            }
            _ => None,
        })
    };
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    assert_eq!(text(&tab.engine), "Slowfirstcaf\u{e9}");
    assert_eq!(tab.engine.shown.page.encoding(), encoding::WINDOWS_1252);
}

#[test]
fn stopping_a_streamed_load_keeps_what_came() {
    let server = slow_page(FIRST, "<p>after</p>", Duration::from_secs(20));
    let page = server.url("/page.html");
    let mut tab = Tab::new();
    tab.engine.navigate(page.clone());
    tab.wait_until(|engine| engine.navigation.is_none());
    tab.engine.stop();
    tab.wait_until(|engine| engine.shown.parsing.is_none());
    assert_eq!(text(&tab.engine), "Slowfirst");
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
}

#[test]
fn a_late_meta_before_the_body_starts_the_document_again_before_it_is_shown() {
    // Past the prescan and before any body: the restart is the incoming
    // document's, which starts empty again.
    let mut page = b"<head><title>caf\xE9</title><!--".to_vec();
    page.extend(vec![b'.'; 1100]);
    page.extend_from_slice(b"--><meta charset=windows-1252></head><body><p>x");
    let server = serve(move |path| match path {
        "/page.html" => Some(Reply::ok("text/html", &page)),
        _ => None,
    });
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    assert_eq!(tab.engine.title(), "caf\u{e9}");
    let document = tab.engine.document();
    let titles = document
        .subtree(document.root())
        .filter(|&n| document.tag(n) == Some("title"))
        .count();
    assert_eq!(titles, 1);
}

#[test]
fn a_large_document_is_parsed_a_slice_a_poll_and_loads() {
    // 60,000 tokens arriving in one piece, from a `data:` URL: many
    // slices, between which only the engine wakes itself.
    let url = Url::parse(&format!("data:text/html,{}", "<p>x</p>".repeat(20_000))).unwrap();
    let mut tab = Tab::new();
    load_all(&mut tab, url);
    assert_eq!(text(&tab.engine).len(), 20_000);
    assert!(tab.engine.shown.parsing.is_none());
}

#[test]
fn a_sheet_that_comes_while_the_body_still_arrives_leaves_the_page_loading() {
    let server = {
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{FIRST}<link rel=stylesheet href=late.css>",
            FIRST.len() + "<link rel=stylesheet href=late.css>".len() + "<p>after</p>".len()
        );
        serve(move |path| match path {
            "/page.html" => Some(Reply::Pieces(vec![
                (Duration::ZERO, head.clone().into_bytes()),
                (Duration::from_millis(1500), b"<p>after</p>".to_vec()),
            ])),
            "/late.css" => Some(css(".late {}")),
            _ => None,
        })
    };
    let mut tab = Tab::new();
    tab.engine.navigate(server.url("/page.html"));
    tab.wait_until(|engine| engine.navigation.is_none() && engine.shown.page.settled());
    assert_eq!(tab.names(), ["late"]);
    assert_eq!(
        tab.engine.load_state(),
        &LoadState::Loading,
        "the body is still coming"
    );
    load_all(&mut tab, server.url("/page.html"));
}

#[test]
fn a_document_left_while_it_parses_is_not_kept() {
    // All of it fetched, but far from parsed: 60,000 tokens from a
    // `data:` URL, which asks for no icon, so that nothing is in flight;
    // left for a page the browser makes at once, before another slice.
    let big = Url::parse(&format!("data:text/html,{}", "<p>x</p>".repeat(20_000))).unwrap();
    let silver = Url::parse("about:silver").unwrap();
    let mut tab = Tab::new();
    tab.engine.navigate(big.clone());
    tab.wait_until(|engine| {
        engine.navigation.is_none()
            && engine.shown.parsing.is_some()
            && engine.network.in_flight() == 0
    });
    tab.engine.navigate(silver.clone());
    assert_eq!(tab.engine.url(), Some(&silver));
    assert_eq!(tab.engine.history.kept_len(), 0, "not in the cache");
    // Parsed whole, it is kept.
    load_all(&mut tab, big);
    tab.engine.navigate(silver);
    assert_eq!(tab.engine.history.kept_len(), 1);
}

#[test]
fn a_document_comes_in_pieces_as_its_body_arrives() {
    let first = "<!DOCTYPE html><p>first</p>";
    let rest = "<p>after</p>";
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{first}",
        first.len() + rest.len()
    );
    let server = serve(move |path| match path {
        "/page.html" => Some(Reply::Pieces(vec![
            (Duration::ZERO, head.clone().into_bytes()),
            // Long enough that the first piece is surely taken alone.
            (Duration::from_millis(1500), rest.as_bytes().to_vec()),
        ])),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.navigate(server.url("/page.html"));
    // The head and the first piece, before the rest is sent.
    tab.wait_until(|engine| {
        matches!(
            incoming(engine),
            Some(Incoming::Receiving { body, .. }) if body == first.as_bytes()
        )
    });
    assert_eq!(tab.engine.load_state(), &LoadState::Loading);
    load_all(&mut tab, server.url("/page.html"));
    assert_eq!(text(&tab.engine), "firstafter");
}

#[test]
fn a_document_arrives_with_no_change_pending() {
    // Built by the parser into the arena: what it built is the document,
    // not a change to it.
    let server = serve(|path| match path {
        "/page.html" => Some(html("<p>a</p><p>b</p>")),
        _ => None,
    });
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    assert!(!tab.engine.shown.document.has_changes());
}

#[test]
fn an_image_is_fetched_once_read_and_waited_for() {
    use crate::image_store::ImageState;
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<body><img src=a.png><img src=a.png><img src=b.png><img src=''><img></body>",
        )),
        "/a.png" => Some(Reply::ok("image/png", &png_bytes(4, 3))),
        "/b.png" => Some(Reply::ok("image/png", &png_bytes(2, 2))),
        _ => None,
    });
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    match image_state(&tab.engine, &server.url("/a.png")) {
        Some(ImageState::Ready(ready)) => {
            assert_eq!((ready.header.width, ready.header.height), (4, 3));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        image_state(&tab.engine, &server.url("/b.png")),
        Some(ImageState::Ready(_))
    ));
    // Each URL once.
    let mut fetched: Vec<String> = server
        .requests()
        .into_iter()
        .map(|request| request.target)
        .filter(|target| target.ends_with(".png"))
        .collect();
    fetched.sort_unstable();
    assert_eq!(fetched, ["/a.png", "/b.png"]);
    // An empty `src` fetches nothing, not the page again.
    let pages = server
        .requests()
        .into_iter()
        .filter(|request| request.target == "/page.html");
    assert_eq!(pages.count(), 1);
}

// SVG images (M32): read by the XML parser, sized by their attributes,
// drawn at the size they land on.

/// An SVG document filled red, its root's attributes `attrs`.
fn svg_image(attrs: &str) -> Vec<u8> {
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' {attrs}>\
         <rect width='100%' height='100%' fill='red'/></svg>"
    )
    .into_bytes()
}

/// The size each replaced fragment of an `img` was laid out at, by the
/// `img`'s id.
fn img_sizes(engine: &mut Engine) -> Vec<(String, f32, f32)> {
    fn walk(engine: &Engine, fragments: &[layout::Fragment], out: &mut Vec<(String, f32, f32)>) {
        for f in fragments {
            if let layout::FragmentKind::Replaced(_) = f.kind
                && let Some(id) = engine.document().attr(f.node, "id")
            {
                out.push((id.to_owned(), f.w, f.h));
            }
            walk(engine, &f.children, out);
        }
    }
    let fragments = engine.fragments(800.0);
    let mut out = Vec::new();
    walk(engine, &fragments, &mut out);
    out
}

#[test]
fn an_svg_image_is_sized_as_the_reference_browser_sizes_it() {
    use crate::image_store::ImageState;
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<body style='margin: 0'><img id=a src=a.svg><img id=b src=b.svg>\
             <img id=c src=c.svg><img id=d src=d.svg>\
             <div style='width: 800px'><img id=e src=e.svg></div>\
             <img id=f src=f.svg><img id=g src=g.svg><img id=h src=h.svg>\
             <img id=i src=i.svg></body>",
        )),
        "/a.svg" => Some(Reply::ok(
            "image/svg+xml",
            &svg_image("width='40' height='20'"),
        )),
        "/b.svg" => Some(Reply::ok(
            "image/svg+xml",
            &svg_image("width='2em' height='1in'"),
        )),
        "/c.svg" => Some(Reply::ok("image/svg+xml", &svg_image(""))),
        "/d.svg" => Some(Reply::ok("image/svg+xml", &svg_image("width='50%'"))),
        "/e.svg" => Some(Reply::ok("image/svg+xml", &svg_image("viewBox='0 0 2 1'"))),
        "/f.svg" => Some(Reply::ok(
            "image/svg+xml",
            b"<svg xmlns='http://www.w3.org/2000/svg'>",
        )),
        "/g.svg" => Some(Reply::ok("image/svg+xml", b"<svg width='4' height='4'/>")),
        "/h.svg" => Some(Reply::ok(
            "image/svg+xml",
            b"<g xmlns='http://www.w3.org/2000/svg' width='4'/>",
        )),
        // Typed nothing an image is known by: sniffed.
        "/i.svg" => Some(Reply::ok(
            "application/octet-stream",
            &svg_image("width='7' height='9'"),
        )),
        _ => None,
    });
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    let header = |name: &str| match image_state(&tab.engine, &server.url(name)) {
        Some(ImageState::Ready(ready)) => Some((ready.header.width, ready.header.height)),
        _ => None,
    };
    assert_eq!(header("/a.svg"), Some((40, 20)));
    assert_eq!(header("/b.svg"), Some((32, 96)), "2em, 1in");
    assert_eq!(header("/c.svg"), Some((300, 150)));
    assert_eq!(
        header("/d.svg"),
        Some((300, 150)),
        "a percentage is no size"
    );
    assert_eq!(
        header("/e.svg"),
        Some((300, 150)),
        "a ratio alone reads 300 by 150"
    );
    for broken in ["/f.svg", "/g.svg", "/h.svg"] {
        assert_eq!(
            image_state(&tab.engine, &server.url(broken)),
            Some(ImageState::Broken),
            "{broken}"
        );
    }
    assert_eq!(header("/i.svg"), Some((7, 9)));
    let sizes = img_sizes(&mut tab.engine);
    let size = |id: &str| {
        sizes
            .iter()
            .find(|(i, ..)| i == id)
            .map(|&(_, w, h)| (w, h))
    };
    assert_eq!(size("a"), Some((40.0, 20.0)));
    assert_eq!(size("c"), Some((300.0, 150.0)));
    assert_eq!(
        size("e"),
        Some((800.0, 400.0)),
        "laid out by its ratio alone"
    );
}

#[test]
fn an_svg_image_is_drawn_at_the_size_it_lands_on() {
    use crate::image_store::{ImageState, Pixels};
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<body style='margin: 0'><img src=a.svg style='width: 200px; height: 100px'>\
             <div style='width: 50px; height: 50px; background: url(a.svg) 0 0 / 25px 25px'>\
             </div></body>",
        )),
        "/a.svg" => Some(Reply::ok(
            "image/svg+xml",
            b"<svg xmlns='http://www.w3.org/2000/svg' width='20' height='10'>\
              <style>rect { fill: blue }</style><rect width='10' height='10'/></svg>",
        )),
        _ => None,
    });
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    // The first frame asks for the drawing; the next shows it.
    tab.engine.frame(300, 200);
    tab.wait_until(|engine| !engine.shown.images.decoding());
    let frame = tab.engine.frame(300, 200);
    let at = |x: usize, y: usize| {
        let i = (y * 300 + x) * 4;
        [frame[i], frame[i + 1], frame[i + 2]]
    };
    // Its own sheet styles it; its left half blue, scaled ten times.
    assert_eq!(at(50, 50), [0, 0, 255]);
    assert_eq!(at(150, 50), [255, 255, 255]);
    match image_state(&tab.engine, &server.url("/a.svg")) {
        Some(ImageState::Ready(ready)) => match &ready.pixels {
            Pixels::Decoded(image) => assert_eq!((image.width, image.height), (200, 100)),
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    }
}

#[test]
fn what_is_no_image_is_broken() {
    use crate::image_store::ImageState;
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<body><img src=missing.png><img src=text.png><img src=empty.png>\
             <img src=picture.svg></body>",
        )),
        "/text.png" => Some(html("not an image")),
        "/empty.png" => Some(Reply::ok("image/png", b"")),
        // Since M32 an image; one that is no SVG document is broken.
        "/picture.svg" => Some(Reply::ok("image/svg+xml", b"<html/>")),
        _ => None,
    });
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    for path in ["/missing.png", "/text.png", "/empty.png", "/picture.svg"] {
        assert_eq!(
            image_state(&tab.engine, &server.url(path)),
            Some(ImageState::Broken),
            "{path}"
        );
    }
}

#[test]
fn the_load_waits_for_an_image_whose_arrival_lays_out_again() {
    let gate = Gate::new();
    let held = gate.clone();
    let server = serve(move |path| match path {
        "/page.html" => Some(html("<body><p>Some text</p><img src=slow.png></body>")),
        "/slow.png" => {
            held.wait();
            Some(Reply::ok("image/png", &png_bytes(3, 3)))
        }
        _ => None,
    });
    let page = server.url("/page.html");
    let mut tab = Tab::new();
    tab.engine.navigate(page.clone());
    tab.wait_until(|engine| incoming(engine).is_none() && engine.url() == Some(&page));
    assert_eq!(tab.engine.load_state(), &LoadState::Loading);
    tab.engine.frame(200, 100);
    let runs = tab.engine.layout_runs();
    tab.engine.frame(200, 100);
    assert_eq!(tab.engine.layout_runs(), runs, "nothing changed yet");
    gate.open();
    tab.wait_until(|engine| engine.load_state() == &LoadState::Loaded);
    tab.engine.frame(200, 100);
    assert_eq!(tab.engine.layout_runs(), runs + 1);
}

#[test]
fn an_img_added_later_is_fetched() {
    use crate::image_store::ImageState;
    let server = serve(|path| match path {
        "/page.html" => Some(html("<body><p>Nothing yet</p></body>")),
        "/c.png" => Some(Reply::ok("image/png", &png_bytes(5, 1))),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/page.html"));
    tab.engine.mutate(|document| {
        let body = document
            .subtree(document.root())
            .find(|&node| document.tag(node) == Some("body"))
            .unwrap();
        let img = document.create_element("img").unwrap();
        document.set_attribute(img, "src", "c.png").unwrap();
        document.append_child(body, img).unwrap();
    });
    let url = server.url("/c.png");
    tab.wait_until(|engine| matches!(image_state(engine, &url), Some(ImageState::Ready(_))));
}

/// The element of the shown document whose `id` is `id`.
fn by_id(engine: &Engine, id: &str) -> dom::NodeId {
    let document = &engine.shown.document;
    document
        .subtree(document.root())
        .find(|&n| document.attr(n, "id") == Some(id))
        .unwrap_or_else(|| panic!("no element #{id}"))
}

/// The kinds of the fragments of the element `#id` in a layout 800px
/// wide, and the size of its first box: `(w, h)`.
fn img_box(engine: &Engine, id: &str) -> (Vec<String>, Option<(f32, f32)>) {
    let node = by_id(engine, id);
    let fragments: Vec<layout::Fragment> = engine
        .fragments(800.0)
        .into_iter()
        .filter(|f| f.node == node)
        .collect();
    let kinds = fragments
        .iter()
        .map(|f| match &f.kind {
            layout::FragmentKind::Box => "box".to_string(),
            layout::FragmentKind::InlineBox => "inline".to_string(),
            layout::FragmentKind::Replaced(r) => format!("{r:?}")
                .split([' ', '{'])
                .next()
                .unwrap_or_default()
                .to_string(),
            layout::FragmentKind::Text(t) => format!("text {t}"),
            layout::FragmentKind::Marker(_) => "marker".to_string(),
        })
        .collect();
    let size = fragments
        .iter()
        .find(|f| matches!(f.kind, layout::FragmentKind::Box))
        .map(|f| (f.w, f.h));
    (kinds, size)
}

#[test]
fn an_img_shows_its_image_its_icon_or_its_alt_text() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<body><img id=a src=a.png><img id=b src=missing.png>\
             <p><img id=c src=missing.png alt=Hi></p><img id=d alt=''></body>",
        )),
        "/a.png" => Some(Reply::ok("image/png", &png_bytes(40, 30))),
        _ => None,
    });
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    let engine = &tab.engine;
    assert_eq!(img_box(engine, "a").1, Some((40.0, 30.0)));
    assert!(img_box(engine, "a").0.contains(&"Image".to_string()));
    assert_eq!(img_box(engine, "b").1, Some((16.0, 16.0)));
    assert!(img_box(engine, "b").0.contains(&"Icon".to_string()));
    let (kinds, _) = img_box(engine, "c");
    assert!(kinds.contains(&"inline".to_string()), "{kinds:?}");
    assert!(kinds.contains(&"text Hi".to_string()), "{kinds:?}");
    assert_eq!(img_box(engine, "d").1, Some((0.0, 0.0)));
}

#[test]
fn a_frame_draws_an_image_once_its_pixels_are_decoded() {
    use crate::image_store::{ImageState, Pixels};
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<body style='margin: 0'><img src=a.png style='display: block'></body>",
        )),
        "/a.png" => Some(Reply::ok("image/png", &png_bytes(40, 30))),
        _ => None,
    });
    let url = server.url("/a.png");
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    let at = |frame: &[u8], x: usize, y: usize| {
        let i = (y * 100 + x) * 4;
        [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
    };
    // The first frame asks for the pixels and draws nothing in their
    // place.
    let first = tab.engine.frame(100, 100);
    assert_eq!(at(&first, 10, 10), [255; 4]);
    tab.wait_until(|engine| {
        matches!(
            image_state(engine, &url),
            Some(ImageState::Ready(ready)) if matches!(ready.pixels, Pixels::Decoded(_))
        )
    });
    assert!(tab.engine.needs_frame());
    // The next draws them: 200 grey at 200 alpha over white.
    let second = tab.engine.frame(100, 100);
    assert_eq!(at(&second, 10, 10), [212, 212, 212, 255]);
    assert_eq!(at(&second, 39, 29), [212, 212, 212, 255]);
    assert_eq!(at(&second, 40, 10), [255; 4]);
    assert_eq!(at(&second, 10, 30), [255; 4]);
}

#[test]
fn an_imgs_new_src_or_alt_is_laid_out_again() {
    let server = serve(|path| match path {
        "/page.html" => Some(html("<body><img id=a src=a.png></body>")),
        "/a.png" => Some(Reply::ok("image/png", &png_bytes(40, 30))),
        "/b.png" => Some(Reply::ok("image/png", &png_bytes(20, 10))),
        _ => None,
    });
    let b = server.url("/b.png");
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    tab.engine.frame(100, 100);
    assert_eq!(img_box(&tab.engine, "a").1, Some((40.0, 30.0)));
    let a = by_id(&tab.engine, "a");
    tab.engine
        .mutate(|document| document.set_attribute(a, "src", "b.png"))
        .unwrap();
    // Coming: nothing, laid out at once.
    tab.engine.frame(100, 100);
    assert_eq!(img_box(&tab.engine, "a").1, Some((0.0, 0.0)));
    tab.wait_until(|engine| {
        matches!(
            image_state(engine, &b),
            Some(crate::image_store::ImageState::Ready(_))
        )
    });
    tab.engine.frame(100, 100);
    let size = |engine: &mut Engine| {
        let node = by_id(engine, "a");
        engine
            .layout(100.0)
            .iter()
            .find(|f| f.node == node && matches!(f.kind, layout::FragmentKind::Box))
            .map(|f| (f.w, f.h))
    };
    assert_eq!(size(&mut tab.engine), Some((20.0, 10.0)));
    // A broken one's alt text, put in, is shown in the layout the frame
    // draws.
    tab.engine
        .mutate(|document| document.set_attribute(a, "src", "missing.png"))
        .unwrap();
    tab.wait_until(|engine| engine.shown.images.settled());
    tab.engine.frame(100, 100);
    assert_eq!(size(&mut tab.engine), Some((16.0, 16.0)));
    tab.engine
        .mutate(|document| document.set_attribute(a, "alt", "Hi"))
        .unwrap();
    tab.engine.frame(100, 100);
    let node = by_id(&tab.engine, "a");
    assert!(
        tab.engine
            .layout(100.0)
            .iter()
            .any(|f| f.node == node && matches!(f.kind, layout::FragmentKind::InlineBox))
    );
}

#[test]
fn a_background_image_is_fetched_against_its_sheet_waited_for_and_drawn() {
    use crate::image_store::{ImageState, Pixels};
    let server = serve(|path| {
        match path {
        "/page.html" => Some(html(
            "<link rel=stylesheet href=css/s.css><body style='margin: 0'>\
             <div style='width: 40px; height: 30px'></div>\
             <p style='height: 4px; background: url(b.png)'></p></body>",
        )),
        "/css/s.css" => Some(Reply::ok(
            "text/css",
            b"div { background: url(a.png) no-repeat } p::before { content: ''; background: url(c.png) }",
        )),
        "/css/a.png" => Some(Reply::ok("image/png", &png_bytes(20, 10))),
        "/b.png" | "/css/c.png" | "/d.png" => Some(Reply::ok("image/png", &png_bytes(4, 4))),
        _ => None,
    }
    });
    let (a, b, c) = (
        server.url("/css/a.png"),
        server.url("/b.png"),
        server.url("/css/c.png"),
    );
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    // Loaded once all are read, a `::before`'s too: no frame needed to
    // find them.
    for url in [&a, &b, &c] {
        assert!(
            matches!(image_state(&tab.engine, url), Some(ImageState::Ready(_))),
            "{url}"
        );
    }
    let at = |frame: &[u8], x: usize, y: usize| {
        let i = (y * 100 + x) * 4;
        [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
    };
    let first = tab.engine.frame(100, 100);
    assert_eq!(at(&first, 5, 5), [255; 4], "nothing until decoded");
    tab.wait_until(|engine| {
        matches!(
            image_state(engine, &a),
            Some(ImageState::Ready(ready)) if matches!(ready.pixels, Pixels::Decoded(_))
        )
    });
    let second = tab.engine.frame(100, 100);
    assert_eq!(at(&second, 5, 5), [212, 212, 212, 255]);
    assert_eq!(at(&second, 19, 9), [212, 212, 212, 255]);
    assert_eq!(at(&second, 25, 5), [255; 4], "not repeated");
    assert_eq!(at(&second, 5, 15), [255; 4]);
    // One a style names later is fetched when the styles are computed.
    let d = server.url("/d.png");
    let document = &tab.engine.shown.document;
    let div = document
        .subtree(document.root())
        .find(|&n| document.tag(n) == Some("div"))
        .expect("the div");
    tab.engine
        .mutate(|document| document.set_attribute(div, "style", "background: url(d.png)"))
        .unwrap();
    tab.engine.frame(100, 100);
    tab.wait_until(|engine| matches!(image_state(engine, &d), Some(ImageState::Ready(_))));
}

#[test]
fn markers_and_contents_images_are_fetched_and_lay_out_again_on_arrival() {
    use crate::image_store::{ImageState, Pixels};
    let gate = Gate::new();
    let held = gate.clone();
    let server = serve(move |path| match path {
        "/page.html" => Some(html(
            "<style>body { margin: 0 } ul { list-style-image: url(m.png) }             #c { content: url(c.png) } #p::after { content: 'x' url(c.png) }</style>             <body><div id=c>replaced</div><ul><li id=li>item</li></ul><p id=p>a</p></body>",
        )),
        "/m.png" => {
            held.wait();
            Some(Reply::ok("image/png", &png_bytes(4, 3)))
        }
        "/c.png" => Some(Reply::ok("image/png", &png_bytes(20, 10))),
        _ => None,
    });
    let (page, c) = (server.url("/page.html"), server.url("/c.png"));
    let mut tab = Tab::new();
    tab.engine.navigate(page.clone());
    tab.wait_until(|engine| {
        incoming(engine).is_none()
            && engine.url() == Some(&page)
            && matches!(image_state(engine, &c), Some(ImageState::Ready(_)))
    });
    assert_eq!(
        tab.engine.load_state(),
        &LoadState::Loading,
        "the marker's to come"
    );
    assert_eq!(
        img_box(&tab.engine, "c"),
        (
            vec!["box".to_string(), "Image".to_string()],
            Some((20.0, 10.0))
        )
    );
    let kinds = img_box(&tab.engine, "p").0;
    assert_eq!(kinds, ["box", "text a", "inline", "text x", "box", "Image"]);
    let marker = |engine: &mut Engine| {
        let li = by_id(engine, "li");
        let layout = engine.layout(200.0);
        layout::flatten(layout.roots.clone())
            .into_iter()
            .filter(|f| {
                f.node == li && f.generated == Some(style::selector_matching::PseudoElement::Marker)
            })
            .map(|f| match f.kind {
                layout::FragmentKind::Replaced(_) => (f.w, f.h),
                _ => (0.0, 0.0),
            })
            .collect::<Vec<_>>()
    };
    tab.engine.frame(200, 100);
    assert_eq!(marker(&mut tab.engine), [(0.0, 0.0)], "the disc for now");
    let runs = tab.engine.layout_runs();
    gate.open();
    tab.wait_until(|engine| engine.load_state() == &LoadState::Loaded);
    assert_eq!(marker(&mut tab.engine), [(4.0, 3.0)]);
    assert_eq!(tab.engine.layout_runs(), runs + 1);
    // Drawn once decoded.
    tab.engine.frame(200, 100);
    tab.wait_until(|engine| {
        matches!(
            image_state(engine, &c),
            Some(ImageState::Ready(ready)) if matches!(ready.pixels, Pixels::Decoded(_))
        )
    });
    let frame = tab.engine.frame(200, 100);
    let i = (5 * 200 + 5) * 4;
    assert_eq!(frame[i..i + 4], [212, 212, 212, 255]);
}

#[test]
fn a_border_image_is_fetched_and_drawn_in_the_borders_place() {
    use crate::image_store::{ImageState, Pixels};
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<body style='margin: 0'><div style='width: 20px; height: 20px;              border: 5px solid red; border-image: url(b.png) 1'></div></body>",
        )),
        "/b.png" => Some(Reply::ok("image/png", &png_bytes(4, 4))),
        _ => None,
    });
    let b = server.url("/b.png");
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    let at = |frame: &[u8], x: usize, y: usize| {
        let i = (y * 100 + x) * 4;
        [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
    };
    let first = tab.engine.frame(100, 100);
    assert_eq!(at(&first, 2, 2), [255; 4], "no border while it decodes");
    tab.wait_until(|engine| {
        matches!(
            image_state(engine, &b),
            Some(ImageState::Ready(ready)) if matches!(ready.pixels, Pixels::Decoded(_))
        )
    });
    let second = tab.engine.frame(100, 100);
    assert_eq!(at(&second, 2, 2), [212, 212, 212, 255]);
    assert_eq!(at(&second, 15, 2), [212, 212, 212, 255]);
    assert_eq!(at(&second, 15, 15), [255; 4], "no fill");
}

/// An APNG 2 by 2, red for 100ms, or `first` seconds as a fraction,
/// then blue for 200ms, played `plays` times.
fn apng_bytes(plays: u32, first: (u16, u16)) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, 2, 2);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_animated(2, plays).unwrap();
    encoder.set_frame_delay(first.0, first.1).unwrap();
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&[255, 0, 0, 255].repeat(4))
        .unwrap();
    writer.set_frame_delay(2, 10).unwrap();
    writer
        .write_image_data(&[0, 0, 255, 255].repeat(4))
        .unwrap();
    writer.finish().unwrap();
    out
}

#[test]
fn an_animated_image_steps_through_its_frames_on_the_documents_timeline() {
    use crate::FrameTime;
    use crate::image_store::{ImageState, Pixels};
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<body style='margin: 0'><img src=a.png style='display: block; width: 10px'>\
             <img src=a.png style='display: block; margin-top: 300px; width: 10px'></body>",
        )),
        "/below.html" => Some(html(
            "<body style='margin: 0'>\
             <img src=b.png style='display: block; margin-top: 150px; width: 10px'></body>",
        )),
        "/two.html" => Some(html(
            "<body style='margin: 0'><img src=a.png><img src=d.png></body>",
        )),
        "/a.png" | "/b.png" | "/c.png" => Some(Reply::ok("image/png", &apng_bytes(2, (1, 10)))),
        "/d.png" => Some(Reply::ok("image/png", &apng_bytes(2, (1, 20)))),
        _ => None,
    });
    let start = Duration::from_secs(1);
    let at = |ms: u64| FrameTime {
        now: start + Duration::from_millis(ms),
        deadline: start + Duration::from_millis(ms + 16),
    };
    let decoded = |url: Url| {
        move |engine: &Engine| {
            matches!(
                image_state(engine, &url),
                Some(ImageState::Ready(ready)) if matches!(ready.pixels, Pixels::Decoded(_))
            )
        }
    };
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    // The timeline starts at its arrival, before the first frame: its
    // time then is 0.
    tab.engine.frame_at(100, 100, at(0));
    tab.wait_until(decoded(server.url("/a.png")));
    let runs = tab.engine.layout_runs();
    let colour_at = |tab: &mut Tab, ms: u64| {
        let frame = tab.engine.frame_at(100, 100, at(ms));
        let i = (5 * 100 + 5) * 4;
        let colour = [frame[i], frame[i + 1], frame[i + 2]];
        (colour, tab.engine.next_frame_at())
    };
    let (red, blue) = ([255, 0, 0], [0, 0, 255]);
    assert_eq!(
        colour_at(&mut tab, 50),
        (red, Some(start + Duration::from_millis(100)))
    );
    assert_eq!(
        colour_at(&mut tab, 150),
        (blue, Some(start + Duration::from_millis(300)))
    );
    let image = render::PixelRect {
        x0: 0,
        y0: 0,
        x1: 10,
        y1: 10,
    };
    assert_eq!(
        tab.engine.redrawn(),
        &render::Redrawn::Rects(vec![image]),
        "the image alone drawn again"
    );
    assert_eq!(
        colour_at(&mut tab, 350),
        (red, Some(start + Duration::from_millis(400)))
    );
    assert!(tab.engine.tick(), "frames to come");
    assert_eq!(
        colour_at(&mut tab, 700),
        (blue, None),
        "played twice, it rests"
    );
    assert_eq!(tab.engine.layout_runs(), runs, "nothing laid out");
    assert!(!tab.engine.tick());
    // One that arrives later starts its own timeline then.
    tab.engine.mutate(|document| {
        let body = document
            .subtree(document.root())
            .find(|&n| document.tag(n) == Some("body"))
            .unwrap();
        let img = document.create_element("img").unwrap();
        document.set_attribute(img, "src", "c.png").unwrap();
        document
            .set_attribute(img, "style", "display: block; width: 10px")
            .unwrap();
        let first = document.first_child(body);
        document.insert_before(body, img, first).unwrap();
    });
    let c = server.url("/c.png");
    tab.wait_until(|engine| matches!(image_state(engine, &c), Some(ImageState::Ready(_))));
    tab.engine.frame_at(100, 100, at(720));
    tab.wait_until(decoded(c));
    assert_eq!(colour_at(&mut tab, 760).0, red);
    // One out of view, listed for the tiles drawn ahead of a scroll,
    // wakes nothing; scrolled into view, it does.
    load_all(&mut tab, server.url("/below.html"));
    tab.engine.frame_at(100, 100, at(0));
    tab.wait_until(decoded(server.url("/b.png")));
    tab.engine.frame_at(100, 100, at(50));
    assert_eq!(tab.engine.next_frame_at(), None);
    tab.engine.set_scroll(60.0);
    tab.engine.frame_at(100, 100, at(60));
    assert_eq!(
        tab.engine.next_frame_at(),
        Some(start + Duration::from_millis(100))
    );
    // Of two, the sooner.
    load_all(&mut tab, server.url("/two.html"));
    tab.engine.frame_at(100, 100, at(0));
    tab.wait_until(decoded(server.url("/a.png")));
    tab.wait_until(decoded(server.url("/d.png")));
    tab.engine.frame_at(100, 100, at(10));
    assert_eq!(
        tab.engine.next_frame_at(),
        Some(start + Duration::from_millis(50))
    );
}

/// The size of the pixels kept for the image at `url`, once decoded.
fn kept_size(engine: &Engine, url: &Url) -> Option<(u32, u32)> {
    use crate::image_store::{ImageState, Pixels};
    match image_state(engine, url) {
        Some(ImageState::Ready(ready)) => match ready.pixels {
            Pixels::Decoded(image) => Some((image.width, image.height)),
            _ => None,
        },
        _ => None,
    }
}

#[test]
fn an_image_drawn_small_is_kept_small_until_drawn_larger() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<body style='margin: 0'><img id=i src=big.png style='width: 10px; height: 10px'>",
        )),
        "/big.png" => Some(Reply::ok("image/png", &png_bytes(100, 50))),
        _ => None,
    });
    let url = server.url("/big.png");
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    tab.engine.frame(100, 100);
    // Drawn 10 by 10, under half of 100 by 50: kept at twice that.
    tab.wait_until(|engine| kept_size(engine, &url).is_some());
    assert_eq!(kept_size(&tab.engine, &url), Some((20, 20)));
    let grey = |frame: &[u8], x: usize, y: usize| frame[(y * 100 + x) * 4];
    assert_eq!(grey(&tab.engine.frame(100, 100), 5, 5), 212);
    // A print has it whole.
    let id = image_id(&tab.engine, &url);
    let whole = tab.engine.shown.images.decoded_now(id).expect("decoded");
    assert_eq!((whole.width, whole.height), (100, 50));
    // Drawn larger, it is decoded again, the copy drawn meanwhile.
    let img = by_id(&tab.engine, "i");
    tab.engine
        .mutate(|document| document.set_attribute(img, "style", "width: 60px; height: 10px"))
        .unwrap();
    assert_eq!(grey(&tab.engine.frame(100, 100), 50, 5), 212);
    tab.wait_until(|engine| kept_size(engine, &url) == Some((100, 50)));
    // Drawn in the frame, it is kept past the budget.
    tab.engine.shown.images.set_budget(0);
    let style = |tab: &mut Tab, css: &str| {
        tab.engine
            .mutate(|document| document.set_attribute(img, "style", css))
            .unwrap();
        tab.engine.frame(100, 100);
    };
    style(&mut tab, "width: 10px; height: 10px");
    assert_eq!(kept_size(&tab.engine, &url), Some((100, 50)));
    // Not drawn, it goes; decoded again for the largest it was drawn at.
    style(&mut tab, "display: none");
    assert_eq!(kept_size(&tab.engine, &url), None);
    style(&mut tab, "width: 10px; height: 10px");
    tab.wait_until(|engine| kept_size(engine, &url).is_some());
    assert_eq!(kept_size(&tab.engine, &url), Some((100, 50)));
}

#[test]
fn past_the_budget_the_pixels_drawn_longest_ago_are_dropped() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<body style='margin: 0; height: 2100px'>\
             <img src=a.png style='position: absolute; top: 0'>\
             <img src=b.png style='position: absolute; top: 1000px'>\
             <img src=c.png style='position: absolute; top: 2000px'></body>",
        )),
        "/a.png" | "/b.png" | "/c.png" => Some(Reply::ok("image/png", &png_bytes(100, 100))),
        _ => None,
    });
    let (a, b, c) = (
        server.url("/a.png"),
        server.url("/b.png"),
        server.url("/c.png"),
    );
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    // Room for two of the three, each 40,000 bytes.
    tab.engine.shown.images.set_budget(100_000);
    for (scroll, url) in [(0.0, &a), (1000.0, &b), (2000.0, &c)] {
        tab.engine.set_scroll(scroll);
        tab.engine.frame(100, 100);
        tab.wait_until(|engine| kept_size(engine, url).is_some());
    }
    tab.engine.frame(100, 100);
    let kept = |engine: &Engine| [&a, &b, &c].map(|url| kept_size(engine, url).is_some());
    assert_eq!(
        kept(&tab.engine),
        [false, true, true],
        "a, drawn first, dropped"
    );
    // Drawn again, decoded again; b, now drawn longest ago, goes.
    tab.engine.set_scroll(0.0);
    tab.engine.frame(100, 100);
    tab.wait_until(|engine| kept_size(engine, &a).is_some());
    tab.engine.frame(100, 100);
    assert_eq!(kept(&tab.engine), [true, false, true]);
}

#[test]
fn an_images_pixels_are_decoded_when_first_asked_for() {
    use crate::image_store::{ImageState, Pixels};
    let server = serve(|path| match path {
        "/page.html" => Some(html("<body><img src=a.png></body>")),
        "/a.png" => Some(Reply::ok("image/png", &png_bytes(4, 3))),
        _ => None,
    });
    let url = server.url("/a.png");
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    let pixels = |engine: &Engine| match image_state(engine, &url) {
        Some(ImageState::Ready(ready)) => ready.pixels,
        other => panic!("{other:?}"),
    };
    assert_eq!(pixels(&tab.engine), Pixels::NotDecoded);
    let id = image_id(&tab.engine, &url);
    assert!(tab.engine.image_pixels(id).is_none());
    assert!(tab.engine.decoding());
    tab.wait_until(|engine| matches!(pixels(engine), Pixels::Decoded(_)));
    assert!(!tab.engine.decoding());
    let image = tab.engine.image_pixels(id).expect("decoded");
    assert_eq!((image.width, image.height, image.frames.len()), (4, 3, 1));
}

/// The red and blue JPEG, 16 by 8, with an EXIF orientation.
fn exif_jpeg(orientation: u16) -> Vec<u8> {
    let jpeg = include_bytes!("../../../images/fixtures/red-blue-16x8.jpg");
    let mut tiff = b"MM\x00\x2a\x00\x00\x00\x08\x00\x01".to_vec();
    tiff.extend([0x01, 0x12, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01]);
    tiff.extend(orientation.to_be_bytes());
    tiff.extend([0; 6]);
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend(tiff);
    let mut out = jpeg[..2].to_vec();
    out.extend([0xFF, 0xE1]);
    out.extend(((app1.len() + 2) as u16).to_be_bytes());
    out.extend(app1);
    out.extend(&jpeg[2..]);
    out
}

#[test]
fn image_orientation_none_leaves_an_image_of_another_origin_turned() {
    let other = serve(|path| match path {
        "/turned.jpg" => Some(Reply::ok("image/jpeg", &exif_jpeg(6))),
        _ => None,
    });
    let elsewhere = other.url("/turned.jpg");
    let server = serve(move |path| match path {
        "/page.html" => Some(html(&format!(
            "<body><img id=same src=turned.jpg style='image-orientation: none'>\
             <img id=other src='{elsewhere}' style='image-orientation: none'>\
             <img id=turned src=turned.jpg></body>"
        ))),
        "/turned.jpg" => Some(Reply::ok("image/jpeg", &exif_jpeg(6))),
        _ => None,
    });
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    let size = |id| img_box(&tab.engine, id).1;
    assert_eq!(size("turned"), Some((8.0, 16.0)));
    assert_eq!(size("same"), Some((16.0, 8.0)), "same origin: as stored");
    assert_eq!(size("other"), Some((8.0, 16.0)), "another origin: turned");
}

#[test]
fn image_orientation_none_leaves_a_background_of_another_origin_turned() {
    let other = serve(|path| match path {
        "/turned.jpg" => Some(Reply::ok("image/jpeg", &exif_jpeg(6))),
        _ => None,
    });
    let elsewhere = other.url("/turned.jpg");
    let server = serve(move |path| match path {
        "/page.html" => Some(html(&format!(
            "<body style='margin: 0'><style>div {{ width: 16px; height: 16px; \
             image-orientation: none; background: no-repeat }}</style>\
             <div style='background-image: url(turned.jpg)'></div>\
             <div style='background-image: url({elsewhere})'></div></body>"
        ))),
        "/turned.jpg" => Some(Reply::ok("image/jpeg", &exif_jpeg(6))),
        _ => None,
    });
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    tab.engine.frame(100, 100);
    tab.wait_until(|engine| !engine.decoding());
    let drawn = tab.engine.frame(100, 100);
    let painted = |x: usize, y: usize| drawn[(y * 100 + x) * 4..][..3] != [255, 255, 255];
    // The same origin's as stored, 16 by 8; the other's turned, 8 by 16.
    assert!(painted(12, 4) && !painted(4, 12), "same origin");
    assert!(!painted(12, 20) && painted(4, 28), "another origin");
}

#[test]
fn a_page_left_while_an_image_decodes_is_not_kept() {
    let server = serve(|path| match path {
        "/a.html" => Some(html("<body><img src=a.png></body>")),
        "/b.html" => Some(html("<body><p>B</p></body>")),
        "/a.png" => Some(Reply::ok("image/png", &png_bytes(4, 3))),
        _ => None,
    });
    let pages = |server: &TestServer| {
        let requests = server.requests().into_iter();
        requests
            .filter(|request| request.target == "/a.html")
            .count()
    };
    let a = server.url("/a.html");
    let mut tab = Tab::new();
    // Left with its image read: kept, and shown again from the cache;
    // `b`, left with nothing in flight, kept too.
    load_all(&mut tab, a.clone());
    tab.settle();
    load_all(&mut tab, server.url("/b.html"));
    tab.settle();
    tab.engine.go_back();
    tab.wait_until(|engine| engine.url() == Some(&a));
    assert_eq!(pages(&server), 1);
    // Left with its pixels being decoded, for the page the cache kept,
    // which is shown at once, before any result is taken: not kept, and
    // fetched again.
    let id = image_id(&tab.engine, &server.url("/a.png"));
    assert!(tab.engine.image_pixels(id).is_none());
    tab.engine.go_forward();
    assert_eq!(tab.engine.url(), Some(&server.url("/b.html")));
    tab.engine.go_back();
    tab.wait_until(|engine| engine.url() == Some(&a) && engine.load_state() == &LoadState::Loaded);
    assert_eq!(pages(&server), 2);
}
