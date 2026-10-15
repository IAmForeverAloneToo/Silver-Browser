//! Frames (M33.2, `docs/design/hardening.md`): an `iframe`'s document
//! loaded by an engine of its own, laid out as a replaced box, drawn
//! into its parent, waited for by its parent's load, and kept within
//! the tab's limits.

use std::time::Duration;

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
fn the_parents_load_comes_when_a_frames_navigation_ends_with_no_document() {
    let gate = Gate::new();
    let held = gate.clone();
    let server = serve(move |path| match path {
        "/page.html" => Some(html(
            "<iframe src=nothing></iframe>\
             <script>onload = () => console.log('page load')</script>",
        )),
        "/nothing" => {
            held.wait();
            Some(Reply::with_fields(204, &[], b""))
        }
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    // All else is done, and the frame's element was told of its first
    // document: the frame's navigation alone holds the load.
    tab.wait_until(|engine| {
        engine.shown.parsing.is_none()
            && !engine.shown.frames.is_empty()
            && engine
                .shown
                .frames
                .values()
                .all(|frame| frame.told.is_some())
    });
    assert_eq!(*tab.engine.load_state(), LoadState::Loading);
    // It ends with no new document to tell the element of.
    gate.open();
    tab.run_until(|lines| lines.iter().any(|line| line == "page load"));
}

#[test]
fn an_iframe_hears_load_for_each_document_whatever_navigated_its_frame() {
    // Its parent's form and link, its `src` and its own script each bring
    // a document, and each `load`; a fragment brings none (found by the
    // adversarial site, M33.8).
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<iframe name=sink></iframe>\
             <form action=/child.html method=post target=sink></form>\
             <a href=/child.html?link target=sink></a>\
             <script>\
             const frame = document.querySelector('iframe');\
             const steps = [\
               () => document.querySelector('form').submit(),\
               () => document.querySelector('a').click(),\
               () => frame.src = '/child.html?src',\
               () => {\
                 frame.src = '/child.html?src#here';\
                 setTimeout(() => frame.src = '/child.html?self', 200);\
               },\
             ];\
             onload = () => {\
               frame.onload = () => {\
                 console.log('load ' + steps.length);\
                 setTimeout(() => steps.length && steps.shift()(), 50);\
               };\
               steps.shift()();\
             };\
             </script>",
        )),
        _ if path.starts_with("/child.html") => Some(html(
            "<script>onload = () => {\
             if (location.search == '?self') location.href = '/child.html?itself';\
             };</script>",
        )),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    let lines =
        tab.run_until(|lines| lines.iter().filter(|line| line.starts_with("load")).count() >= 5);
    // Form, link, `src`: one each; the fragment none; the frame's own
    // navigation one more after the `src` that started it.
    assert_eq!(lines, ["load 3", "load 2", "load 1", "load 0", "load 0"]);
    let targets: Vec<String> = server
        .requests()
        .into_iter()
        .filter(|request| request.target.starts_with("/child.html"))
        .map(|request| format!("{} {}", request.method, request.target))
        .collect();
    assert_eq!(
        targets,
        [
            "POST /child.html",
            "GET /child.html?link",
            "GET /child.html?src",
            "GET /child.html?self",
            "GET /child.html?itself",
        ]
    );
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
    // As the frames target counts them (M33.6).
    assert_eq!(
        crate::differential::frames_of(&tab.engine),
        (MAX_FRAME_DEPTH, MAX_FRAME_DEPTH)
    );
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
    // Its frame is the window's again.
    let tree = frames::lock(&tab.engine.frame.tree);
    assert_eq!(tree.children(frames::TOP).len(), 1);
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

// ---- Step 2: the pointer, the wheel, focus and keys in frames ----

/// A tab showing `parent`, scripts on, its frames loaded and drawn at
/// 800 by 600; the console taken.
fn framed(parent: &str) -> Tab {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.load_document(parent);
    settle_frames(&mut tab);
    tab.engine
        .frame_at(800, 600, crate::FrameTime::at(Duration::ZERO));
    tab.engine.take_console();
    tab
}

/// The console's lines since it was last taken.
fn console(tab: &mut Tab) -> Vec<String> {
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

/// A click of the primary button at `(x, y)`.
fn click(tab: &mut Tab, x: f32, y: f32) {
    tab.engine.pointer_move(x, y);
    tab.engine.pointer_down(x, y, crate::Button::Primary, 1);
    tab.engine.pointer_up(x, y, crate::Button::Primary, 1);
}

#[test]
fn a_click_in_a_frame_is_the_frames_and_focuses_its_iframe() {
    let mut tab = framed(
        "<body style='margin:0' onclick=\"console.log('parent click')\">\
         <iframe style='border:0; margin: 20px' srcdoc=\"<body style='margin:0'>\
         <button style='display:block; width:100px; height:50px' \
         onmousedown='console.log(event.clientX, event.clientY)' \
         onclick='console.log(this.tagName)'>b</button>\"></iframe>",
    );
    click(&mut tab, 30.0, 25.0);
    assert_eq!(
        console(&mut tab),
        ["[frame about:srcdoc] 10 5", "[frame about:srcdoc] BUTTON"]
    );
    assert_eq!(tab.engine.focused(), Some(iframes(&tab.engine)[0]));
    // Outside the frame, the parent's.
    click(&mut tab, 500.0, 100.0);
    assert_eq!(console(&mut tab), ["parent click"]);
}

#[test]
fn a_transformed_frame_takes_the_point_in_its_own_coordinates() {
    let mut tab = framed(
        "<body style='margin:0'><iframe style='border:0; transform: scale(2); \
         transform-origin: 0 0' srcdoc=\"<body style='margin:0; height:150px' \
         onmousedown='console.log(event.clientX, event.clientY)'>\"></iframe>",
    );
    click(&mut tab, 60.0, 40.0);
    assert_eq!(console(&mut tab), ["[frame about:srcdoc] 30 20"]);
}

#[test]
fn the_pointer_hovers_in_a_frame_and_leaves_it() {
    let mut tab = framed(
        "<body style='margin:0'><iframe style='border:0' srcdoc=\"<body style='margin:0'>\
         <div style='height:100px'></div>\"></iframe>",
    );
    let hovered = |engine: &Engine| {
        let child = frame(engine, 0);
        let document = child.document();
        document
            .subtree(document.root())
            .filter(|&node| document.state(node).contains(dom::ElementState::HOVER))
            .filter_map(|node| document.tag(node).map(str::to_owned))
            .collect::<Vec<_>>()
    };
    tab.engine.pointer_move(10.0, 10.0);
    assert_eq!(hovered(&tab.engine), ["html", "body", "div"]);
    let iframe = iframes(&tab.engine)[0];
    assert!(
        tab.engine
            .document()
            .state(iframe)
            .contains(dom::ElementState::HOVER)
    );
    tab.engine.pointer_move(500.0, 400.0);
    assert!(hovered(&tab.engine).is_empty(), "left");
}

#[test]
fn a_press_in_a_frame_comes_up_in_it_wherever_it_is_released() {
    let mut tab = framed(
        "<body style='margin:0' onmouseup=\"console.log('parent up')\">\
         <iframe style='border:0' srcdoc=\"<body style='margin:0; height:100px'>\
         <script>onmouseup = e => console.log(e.clientX)</script>\"></iframe>",
    );
    tab.engine.pointer_move(10.0, 10.0);
    tab.engine
        .pointer_down(10.0, 10.0, crate::Button::Primary, 1);
    tab.engine.pointer_move(600.0, 10.0);
    tab.engine
        .pointer_up(600.0, 10.0, crate::Button::Primary, 1);
    assert_eq!(console(&mut tab), ["[frame about:srcdoc] 600"]);
    // The next press is the parent's again.
    click(&mut tab, 600.0, 100.0);
    assert_eq!(console(&mut tab), ["parent up"]);
}

#[test]
fn the_wheel_scrolls_a_frame_and_then_its_parent() {
    let mut tab = framed(
        "<body style='margin:0; height:2000px'><iframe style='border:0' \
         srcdoc=\"<body style='margin:0; height:250px'>\"></iframe>",
    );
    tab.engine.wheel(10.0, 10.0, 60.0);
    let child_target = |engine: &Engine| frame(engine, 0).shown.scroll_target;
    assert_eq!(child_target(&tab.engine), 60.0);
    assert_eq!(tab.engine.shown.scroll_target, 0.0);
    // The frame holds 100 more; past them the parent takes the wheel.
    tab.engine.wheel(10.0, 10.0, 60.0);
    assert_eq!(child_target(&tab.engine), 100.0);
    tab.engine.wheel(10.0, 10.0, 60.0);
    assert_eq!(child_target(&tab.engine), 100.0);
    assert_eq!(tab.engine.shown.scroll_target, 60.0);
}

#[test]
fn links_and_cursors_in_a_frame_are_its_documents() {
    let server = two_pages(
        "<body style='margin:0'><iframe style='border:0' src=child.html></iframe>",
        "<body style='margin:0'><a href=x.html style='display:block; height:40px; \
         cursor: pointer'>x</a>",
    );
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    settle_frames(&mut tab);
    tab.engine.frame(800, 600);
    let link = tab.engine.link_at(10.0, 10.0).expect("a link");
    assert_eq!(link.url(), &server.url("/x.html"));
    assert_eq!(tab.engine.cursor_at(10.0, 10.0), crate::Cursor::Pointer);
    assert_eq!(tab.engine.cursor_at(10.0, 100.0), crate::Cursor::Default);
}

#[test]
fn keys_and_text_go_to_the_frame_focused() {
    let mut tab = framed(
        "<body style='margin:0'><iframe style='border:0; margin-left:100px' \
         srcdoc=\"<body style='margin:0'><input style='display:block'>\"></iframe>",
    );
    click(&mut tab, 110.0, 10.0);
    assert!(tab.engine.insert_text("ab"));
    let key = |name: &str| crate::Key {
        key: name.to_owned(),
        code: name.to_owned(),
        text: None,
        location: 0,
        repeat: false,
    };
    assert_eq!(
        tab.engine.key_down(&key("Backspace")),
        crate::KeyOutcome::Taken
    );
    tab.engine.key_up(&key("Backspace"));
    let child = frame(&tab.engine, 0);
    let document = child.document();
    let input = document
        .subtree(document.root())
        .find(|&node| document.tag(node) == Some("input"))
        .expect("an input");
    assert_eq!(dom::forms::value(document, input), "a");
    // The caret where the frame's content box lands here.
    let (x, _, _, _) = tab.engine.caret_area().expect("a caret");
    assert!((100.0..150.0).contains(&x), "{x}");
}

#[test]
fn tab_goes_through_a_frames_areas_and_passes_an_empty_one() {
    let mut tab = framed(
        "<input id=a><iframe srcdoc=\"<input id=b><input id=c>\"></iframe>\
         <iframe srcdoc='<p>nothing</p>'></iframe><input id=d>",
    );
    let at = |engine: &Engine| {
        let focused = engine.focused()?;
        if engine.document().tag(focused) == Some("iframe") {
            let child = &engine.shown.frames.get(&focused)?.engine;
            let inner = child.focused()?;
            return child.document().attr(inner, "id").map(str::to_owned);
        }
        engine.document().attr(focused, "id").map(str::to_owned)
    };
    let mut seen = Vec::new();
    while tab.engine.focus_next(false) {
        seen.push(at(&tab.engine).unwrap_or_default());
    }
    assert_eq!(seen, ["a", "b", "c", "d"]);
    let mut back = Vec::new();
    while tab.engine.focus_next(true) {
        back.push(at(&tab.engine).unwrap_or_default());
    }
    assert_eq!(back, ["d", "c", "b", "a"]);
}

#[test]
fn a_frames_window_hears_the_focus_come_and_go() {
    let mut tab = framed(
        "<body style='margin:0'><iframe style='border:0' srcdoc=\"<script>\
         onfocus = () => console.log('focus'); onblur = () => console.log('blur')</script>\
         <input>\"></iframe><p>parent</p>",
    );
    click(&mut tab, 20.0, 20.0);
    assert_eq!(console(&mut tab), ["[frame about:srcdoc] focus"]);
    click(&mut tab, 500.0, 400.0);
    assert_eq!(console(&mut tab), ["[frame about:srcdoc] blur"]);
}

#[test]
fn a_frames_viewport_is_its_box() {
    let mut tab = framed(
        "<body style='margin:0'><iframe style='border:0' srcdoc=\"<script>\
         onresize = () => console.log(innerWidth, innerHeight)</script>\"></iframe>",
    );
    assert_eq!(frame(&tab.engine, 0).viewport(), (300, 150));
    // Frames drawn for a time run the frame's rendering steps.
    let at = |engine: &mut Engine, ms: u64| {
        engine.frame_at(800, 600, crate::FrameTime::at(Duration::from_millis(ms)));
    };
    at(&mut tab.engine, 16);
    let node = iframes(&tab.engine)[0];
    tab.engine.mutate(|document| {
        document
            .set_attribute(node, "style", "border:0; width:200px; height:80px")
            .expect("set");
    });
    at(&mut tab.engine, 32);
    assert_eq!(frame(&tab.engine, 0).viewport(), (200, 80));
    assert_eq!(console(&mut tab), ["[frame about:srcdoc] 200 80"]);
}

#[test]
fn a_frame_prints_as_it_is_drawn() {
    let tab = framed(
        "<body style='margin:0'><iframe style='border:0' \
         srcdoc=\"<body style='background: rgb(255, 0, 0)'>\"></iframe>",
    );
    let pages = tab
        .engine
        .printed_pages(&crate::PrintOptions::default())
        .expect("printed");
    let pixels = pages[0].list.items.iter().find_map(|item| match item {
        paint::DisplayItem::Image { image, .. } if paint::frame_node(image.id).is_some() => {
            image.pixels.clone()
        }
        _ => None,
    });
    let pixels = pixels.expect("the frame's pixels");
    assert_eq!(&pixels.frames[0].pixels[0..4], &[255, 0, 0, 255]);
}

#[test]
fn a_frames_border_and_padding_are_its_parents() {
    // The point in the frame is from its content box; a press on the
    // iframe's border or padding is the parent's.
    let mut tab = framed(
        "<body style='margin:0' onmousedown=\"console.log('parent')\">\
         <iframe style='border: 5px solid; padding: 10px' srcdoc=\"<body style='margin:0; \
         height:150px' onmousedown='console.log(event.clientX, event.clientY)'>\"></iframe>",
    );
    click(&mut tab, 30.0, 40.0);
    assert_eq!(console(&mut tab), ["[frame about:srcdoc] 15 25"]);
    click(&mut tab, 8.0, 40.0);
    assert_eq!(console(&mut tab), ["parent"]);
}

#[test]
fn a_frames_own_navigation_is_made_from_its_container() {
    // A link in a frame of an https: page to http: is mixed content, as
    // the frame's first navigation was: its container's document makes
    // every navigation of the frame's (M33.2).
    let authority = net::test_server::TestAuthority::new();
    let server = TestServer::tls(
        authority.server(&["127.0.0.1"], net::test_server::VALID),
        |request: &ServerRequest| match request.target.as_str() {
            "/page.html" => Reply::ok(
                "text/html",
                b"<iframe srcdoc=\"<a id=l href='http://example.invalid/'>l</a>\
                  <script>onload = () => document.getElementById('l').click()</script>\">\
                  </iframe>",
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
    let deadline = std::time::Instant::now() + WAIT;
    while !matches!(frame(&tab.engine, 0).load_state(), LoadState::Failed(_)) {
        assert!(std::time::Instant::now() < deadline, "the link is followed");
        tab.engine.poll_network();
        tab.engine.run_tasks();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        frame(&tab.engine, 0).load_state(),
        &LoadState::Failed(crate::LoadError::Network(net::NetworkError::MixedContent))
    );
}
