//! Frames (M33.2, `docs/design/hardening.md`): an `iframe`'s document
//! loaded by an engine of its own, laid out as a replaced box, drawn
//! into its parent, waited for by its parent's load, and kept within
//! the tab's limits.

use dom::NodeId;
use net::Url;
use net::test_server::{Reply, ServerRequest, TestServer};

use super::tests::load_all;
use crate::frames::{self, Source};
use crate::testing::*;
use crate::{Engine, LoadState, MAX_FRAME_DEPTH, MAX_FRAMES};

/// The `iframe`s of the engine's document, in tree order.
fn iframes(engine: &Engine) -> Vec<NodeId> {
    let document = engine.document();
    document
        .subtree(document.root())
        .filter(|&node| document.tag(node) == Some("iframe"))
        .collect()
}

/// The engine of the frame of the `index`th `iframe`.
fn frame(engine: &Engine, index: usize) -> &Engine {
    let node = iframes(engine)[index];
    &engine.shown.frames.get(&node).expect("a frame").engine
}

/// The box of the `index`th `iframe` at 800 wide: x, y, w, h.
fn frame_box(engine: &Engine, index: usize) -> (f32, f32, f32, f32) {
    let node = iframes(engine)[index];
    let f = engine
        .fragments(800.0)
        .into_iter()
        .find(|f| f.node == node && matches!(f.kind, layout::FragmentKind::Box))
        .expect("the iframe's box");
    (f.x, f.y, f.w, f.h)
}

/// The pixel at `(x, y)` of a frame `width` wide.
fn pixel(frame: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * width + x) * 4) as usize;
    frame[at..at + 4].try_into().expect("four bytes")
}

/// Polls the tab until its document and its frames have loaded, the
/// frames navigated since its own load too.
fn settle_frames(tab: &mut Tab) {
    tab.wait_until(|engine| {
        *engine.load_state() == LoadState::Loaded && !engine.fetching() && engine.frames_settled()
    });
}

/// A server of `/page.html`, `page`, and of `/child.html`, `child`.
fn two_pages(page: &'static str, child: &'static str) -> TestServer {
    serve(move |path| match path {
        "/page.html" => Some(html(page)),
        "/child.html" => Some(html(child)),
        _ => None,
    })
}

#[test]
fn an_iframe_is_a_replaced_box_of_300_by_150_with_an_inset_border() {
    // As the reference browser lays it out (p1): 304 by 154, a border
    // of 2px inset; its content is never laid out.
    let mut tab = Tab::new();
    tab.engine
        .load_document("<body style='margin:0'><iframe>fallback text</iframe>");
    let (_, _, w, h) = frame_box(&tab.engine, 0);
    assert_eq!((w, h), (304.0, 154.0));
    let node = iframes(&tab.engine)[0];
    let style = tab.engine.styles().get(node).expect("styled").clone();
    assert_eq!(style.border_width, [2.0; 4]);
    assert_eq!(style.border_style, [style::computed::BorderStyle::Inset; 4]);
    assert!(
        !tab.engine.fragments(800.0).iter().any(
            |f| matches!(&f.kind, layout::FragmentKind::Text(text) if text.contains("fallback"))
        ),
        "its fallback text is not shown"
    );
}

#[test]
fn width_height_and_frameborder_are_hints() {
    let mut tab = Tab::new();
    tab.engine.load_document(
        "<body style='margin:0'><iframe width=100 height=50 frameborder=0></iframe>\
         <iframe width=50% frameborder=no></iframe><iframe frameborder=1></iframe>",
    );
    let (_, _, w, h) = frame_box(&tab.engine, 0);
    assert_eq!((w, h), (100.0, 50.0));
    let (_, _, w, h) = frame_box(&tab.engine, 1);
    assert_eq!((w, h), (400.0, 150.0), "a percentage, and no border");
    let (_, _, w, h) = frame_box(&tab.engine, 2);
    assert_eq!((w, h), (304.0, 154.0), "a frameborder of 1 keeps it");
}

#[test]
fn a_srcdoc_is_drawn_into_its_box_and_its_canvas_is_clear() {
    // The frame's document fills the content box; where it gives its
    // canvas no colour, the parent's shows through.
    let mut tab = Tab::new();
    tab.engine.load_document(
        "<body style='margin:0; background: rgb(0, 128, 0)'>\
         <iframe style='border:0' srcdoc=\"<body style='margin:0'>\
         <div style='width:50px; height:50px; background: rgb(255, 0, 0)'></div>\"></iframe>",
    );
    settle_frames(&mut tab);
    let shown = tab.engine.frame(400, 200);
    assert_eq!(
        pixel(&shown, 400, 10, 10),
        [255, 0, 0, 255],
        "the frame's div"
    );
    assert_eq!(
        pixel(&shown, 400, 100, 100),
        [0, 128, 0, 255],
        "clear canvas"
    );
    assert_eq!(
        pixel(&shown, 400, 350, 100),
        [0, 128, 0, 255],
        "past the box"
    );
    // A frame whose body has a colour paints its canvas with it.
    tab.engine.load_document(
        "<body style='margin:0; background: rgb(0, 128, 0)'>\
         <iframe style='border:0' srcdoc=\"<body style='background: rgb(0, 0, 255)'>\"></iframe>",
    );
    settle_frames(&mut tab);
    let shown = tab.engine.frame(400, 200);
    assert_eq!(pixel(&shown, 400, 150, 75), [0, 0, 255, 255]);
}

#[test]
fn a_frame_is_drawn_again_when_its_document_changes() {
    let mut tab = Tab::new();
    tab.engine.load_document(
        "<body style='margin:0'><iframe style='border:0' \
         srcdoc=\"<body style='margin:0; background: rgb(255, 0, 0)'>\"></iframe>",
    );
    settle_frames(&mut tab);
    let shown = tab.engine.frame(400, 200);
    assert_eq!(pixel(&shown, 400, 20, 20), [255, 0, 0, 255]);
    assert!(!tab.engine.needs_frame());
    let node = iframes(&tab.engine)[0];
    let child = &mut tab
        .engine
        .shown
        .frames
        .get_mut(&node)
        .expect("a frame")
        .engine;
    child.mutate(|document| {
        let body = document
            .subtree(document.root())
            .find(|&n| document.tag(n) == Some("body"))
            .expect("a body");
        document
            .set_attribute(body, "style", "margin:0; background: rgb(0, 0, 255)")
            .expect("set");
    });
    assert!(tab.engine.needs_frame(), "the parent wants a frame");
    let shown = tab.engine.frame(400, 200);
    assert_eq!(pixel(&shown, 400, 20, 20), [0, 0, 255, 255]);
}

#[test]
fn a_frames_src_is_fetched_as_an_iframe_by_its_parents_document() {
    let server = two_pages(
        "<body style='margin:0'><iframe style='border:0' src=child.html></iframe>",
        "<body style='margin:0; background: rgb(255, 0, 0)'>",
    );
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    settle_frames(&mut tab);
    let child: Vec<ServerRequest> = server
        .requests()
        .into_iter()
        .filter(|request| request.target == "/child.html")
        .collect();
    assert_eq!(child.len(), 1);
    assert_eq!(child[0].header("Sec-Fetch-Dest"), Some("iframe"));
    assert_eq!(child[0].header("Sec-Fetch-Mode"), Some("navigate"));
    assert_eq!(child[0].header("Sec-Fetch-Site"), Some("same-origin"));
    assert_eq!(child[0].header("Sec-Fetch-User"), None);
    assert_eq!(
        frame(&tab.engine, 0).url(),
        Some(&server.url("/child.html"))
    );
    let shown = tab.engine.frame(400, 200);
    assert_eq!(pixel(&shown, 400, 150, 75), [255, 0, 0, 255]);
}

#[test]
fn the_parents_load_waits_for_its_frames_and_their_loads_come_first() {
    let gate = Gate::new();
    let held = gate.clone();
    let server = serve(move |path| match path {
        "/page.html" => Some(html(
            "<iframe src=child.html onload=\"console.log('iframe load')\"></iframe>\
             <script>onload = () => console.log('page load')</script>",
        )),
        "/child.html" => {
            held.wait();
            Some(html(
                "<script>onload = () => console.log('child load')</script>",
            ))
        }
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    tab.wait_until(|engine| engine.document().subtree(engine.document().root()).count() > 4);
    tab.wait_until(|engine| !iframes(engine).is_empty() && !engine.shown.frames.is_empty());
    assert_eq!(
        *tab.engine.load_state(),
        LoadState::Loading,
        "the frame holds it"
    );
    gate.open();
    let lines = tab.run_until(|lines| lines.iter().any(|line| line == "page load"));
    let url = server.url("/child.html");
    // The tab's console has the document's lines, then each frame's,
    // since it was last taken: within the document's, the frame's load
    // comes first.
    let mut sorted = lines.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        [
            format!("[frame {url}] child load"),
            "iframe load".to_owned(),
            "page load".to_owned(),
        ]
    );
    let at = |line: &str| lines.iter().position(|l| l == line);
    assert!(at("iframe load") < at("page load"), "{lines:?}");
}

#[test]
fn a_srcdoc_takes_its_parents_origin_and_base_url() {
    let server = two_pages(
        "<iframe srcdoc=\"<a href=x.html></a><script>console.log(self.origin, document.URL, \
         document.querySelector('a').href, isSecureContext)</script>\"></iframe>\
         <iframe></iframe>",
        "",
    );
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    settle_frames(&mut tab);
    let origin = server.url("/").origin().serialization();
    let lines: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(
        lines,
        [format!(
            "[frame about:srcdoc] {origin} about:srcdoc {} true",
            server.url("/x.html")
        )]
    );
    // The initial about:blank has its creator's origin too.
    let blank = frame(&tab.engine, 1);
    assert_eq!(blank.url().map(Url::as_str), Some("about:blank"));
    assert_eq!(blank.shown.page.origin(), tab.engine.shown.page.origin());
    assert_eq!(
        frame(&tab.engine, 0).shown.page.origin(),
        tab.engine.shown.page.origin()
    );
}

#[test]
fn a_frame_never_loads_a_document_an_ancestor_shows() {
    // HTML's recursion guard: the page framing itself, the fragment
    // aside, is not loaded again.
    let server = serve(|path| match path {
        "/page.html" => Some(html("<iframe src='page.html#again'></iframe>")),
        _ => None,
    });
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    settle_frames(&mut tab);
    assert_eq!(paths(&server), ["/page.html"]);
    assert_eq!(
        frame(&tab.engine, 0).url().map(Url::as_str),
        Some("about:blank")
    );
}

#[test]
fn frames_nest_to_a_depth_and_a_tab_holds_so_many() {
    // Each level's markup escaped once more inside the last's srcdoc.
    let mut markup = "<p>bottom</p>".to_owned();
    for _ in 0..MAX_FRAME_DEPTH + 2 {
        let escaped = markup.replace('&', "&amp;").replace('"', "&quot;");
        markup = format!("<iframe srcdoc=\"{escaped}\"></iframe>");
    }
    let mut tab = Tab::new();
    tab.engine.load_document(&markup);
    settle_frames(&mut tab);
    let mut depth = 0;
    let mut engine = &tab.engine;
    while let Some(frame) = engine.shown.frames.values().next() {
        depth += 1;
        engine = &frame.engine;
    }
    assert_eq!(depth, MAX_FRAME_DEPTH);
    let lines: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|e| e.text)
        .collect();
    assert!(
        lines
            .iter()
            .any(|line| line.ends_with("An iframe was left empty: frames nest 16 deep at most")),
        "{lines:?}"
    );
    // A tab of more iframes than it may hold frames.
    let mut tab = Tab::new();
    tab.engine
        .load_document(&"<iframe></iframe>".repeat(MAX_FRAMES + 3));
    assert_eq!(tab.engine.shown.frames.len(), MAX_FRAMES);
    assert_eq!(frames::lock(&tab.engine.frame.tree).frames(), MAX_FRAMES);
    let lines: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|e| e.text)
        .collect();
    assert_eq!(
        lines,
        ["An iframe was left empty: a tab has 100 frames at most"; 3]
    );
    // The frames go with their document, and the tree empties.
    tab.engine.load_document("<p>none</p>");
    assert_eq!(frames::lock(&tab.engine.frame.tree).frames(), 0);
}

#[test]
fn an_iframe_taken_out_unloads_its_frame_and_one_navigates_when_its_src_changes() {
    let server = two_pages(
        "<iframe src=child.html></iframe>",
        "<script>onunload = () => console.log('unloaded')</script>",
    );
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    settle_frames(&mut tab);
    let node = iframes(&tab.engine)[0];
    let now = "<script>onunload = () => console.log('gone')</script>";
    tab.engine.mutate(|document| {
        document.set_attribute(node, "srcdoc", now).expect("set");
    });
    settle_frames(&mut tab);
    assert_eq!(
        tab.engine.shown.frames.get(&node).map(|f| &f.source),
        Some(&Source::Srcdoc(now.to_owned()))
    );
    assert_eq!(
        frame(&tab.engine, 0).url().map(Url::as_str),
        Some("about:srcdoc")
    );
    let url = server.url("/child.html");
    let lines: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|e| e.text)
        .collect();
    assert_eq!(lines, [format!("[frame {url}] unloaded")]);
    tab.engine.mutate(|document| {
        let parent = document.parent(node).expect("a parent");
        document.remove_child(parent, node).expect("removed");
    });
    assert!(tab.engine.shown.frames.is_empty());
    assert_eq!(frames::lock(&tab.engine.frame.tree).frames(), 0);
    let lines: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|e| e.text)
        .collect();
    assert_eq!(lines, ["[frame about:srcdoc] gone"]);
}

#[test]
fn a_frame_hears_pagehide_when_its_parent_is_left() {
    // HTML unloads a document's descendants with it: kept by the cache
    // here, so persisted.
    let server = serve(|path| match path {
        "/a.html" => Some(html("<iframe src=child.html></iframe>")),
        "/b.html" => Some(html("<p>b</p>")),
        "/child.html" => Some(html(
            "<script>onpagehide = e => console.log('hide', e.persisted)</script>",
        )),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/a.html"));
    settle_frames(&mut tab);
    load_all(&mut tab, server.url("/b.html"));
    let url = server.url("/child.html");
    let lines: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|e| e.text)
        .collect();
    assert_eq!(lines, [format!("[frame {url}] hide true")]);
}

#[test]
fn a_document_left_while_a_frame_loads_is_not_kept() {
    let gate = Gate::new();
    let held = gate.clone();
    let server = serve(move |path| match path {
        "/a.html" => Some(html("<iframe src=child.html></iframe>")),
        "/b.html" => Some(html("<p>b</p>")),
        "/child.html" => {
            held.wait();
            Some(html("<p>child</p>"))
        }
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.navigate(server.url("/a.html"));
    tab.wait_until(|engine| !engine.shown.frames.is_empty());
    load_all(&mut tab, server.url("/b.html"));
    gate.open();
    assert_eq!(tab.engine.history.kept_len(), 0, "not kept");
    assert_eq!(frames::lock(&tab.engine.frame.tree).frames(), 0);
}

#[test]
fn a_document_kept_by_the_cache_keeps_its_frames() {
    let server = serve(|path| match path {
        "/a.html" => Some(html("<iframe src=child.html></iframe>")),
        "/b.html" => Some(html("<p>b</p>")),
        "/child.html" => Some(html("<p>child</p>")),
        _ => None,
    });
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/a.html"));
    settle_frames(&mut tab);
    load_all(&mut tab, server.url("/b.html"));
    assert_eq!(frames::lock(&tab.engine.frame.tree).frames(), 1, "kept");
    tab.engine.go_back();
    settle_frames(&mut tab);
    assert_eq!(
        frame(&tab.engine, 0).url(),
        Some(&server.url("/child.html"))
    );
    assert_eq!(
        paths(&server),
        ["/a.html", "/child.html", "/b.html"],
        "nothing fetched again"
    );
}

#[test]
fn a_frames_request_is_its_parents_for_mixed_content() {
    // An https: page's frame of an http: URL is blocked, not upgraded,
    // and shows an error page; its element still hears `load`.
    let authority = net::test_server::TestAuthority::new();
    let server = TestServer::tls(
        authority.server(&["127.0.0.1"], net::test_server::VALID),
        |request: &ServerRequest| match request.target.as_str() {
            "/page.html" => Reply::ok(
                "text/html",
                b"<iframe src='http://example.invalid/' onload=\"console.log('load')\"></iframe>",
            ),
            _ => Reply::with_fields(404, &[("Content-Type", "text/plain")], b"no"),
        },
    );
    let mut tab = Tab::new();
    tab.engine.set_network(net::Client::new(
        net::transport::Config::default().trust(authority.root.clone()),
    ));
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    settle_frames(&mut tab);
    let child = frame(&tab.engine, 0);
    assert_eq!(
        child.load_state(),
        &LoadState::Failed(crate::LoadError::Network(net::NetworkError::MixedContent))
    );
    let lines: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|e| e.text)
        .collect();
    assert_eq!(lines, ["load"]);
}

#[test]
fn a_frames_own_history_keeps_no_document() {
    // A frame's navigations of its own are no entries of the tab's, and
    // its back-forward cache holds nothing (M33.2).
    let server = serve(|path| match path {
        "/a.html" => Some(html("<iframe src=one.html></iframe>")),
        "/one.html" => Some(html("<p>one</p>")),
        "/two.html" => Some(html("<p>two</p>")),
        _ => None,
    });
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/a.html"));
    settle_frames(&mut tab);
    let node = iframes(&tab.engine)[0];
    let child = &mut tab
        .engine
        .shown
        .frames
        .get_mut(&node)
        .expect("a frame")
        .engine;
    child.navigate(server.url("/two.html"));
    tab.wait_until(|engine| {
        frame(engine, 0).url() == Some(&server.url("/two.html")) && !engine.fetching()
    });
    let child = frame(&tab.engine, 0);
    assert_eq!(child.history.kept_len(), 0);
    assert!(!tab.engine.can_go_back(), "no entry of the tab's");
}
