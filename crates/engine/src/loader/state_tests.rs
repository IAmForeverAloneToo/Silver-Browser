//! Element state in the engine's documents (M26.4,
//! `docs/design/events.md`): hover from the pointer, with scripts on or
//! off, kept as the page moves under it; `:target` from the URL's
//! fragment; and the cursor over the page.

use dom::{ElementState, NodeId};

use super::script_tests::scripted;
use crate::Cursor;
use crate::testing::*;

/// The element with ID `id`.
fn by_id(tab: &Tab, id: &str) -> NodeId {
    let document = tab.engine.document();
    document
        .subtree(document.root())
        .find(|&node| document.attr(node, "id") == Some(id))
        .unwrap_or_else(|| panic!("#{id}"))
}

/// The IDs of the hovered elements, in tree order.
fn hovered(tab: &Tab) -> Vec<String> {
    let document = tab.engine.document();
    document
        .subtree(document.root())
        .filter(|&node| document.state(node).contains(ElementState::HOVER))
        .map(|node| document.attr(node, "id").unwrap_or("-").to_owned())
        .collect()
}

/// The computed `color` of element `id`, as `rgb(...)`.
fn color(tab: &mut Tab, id: &str) -> String {
    let node = by_id(tab, id);
    let style = tab
        .engine
        .cached_styles()
        .get(node)
        .expect("styled")
        .clone();
    style::serialize::computed_value(&style, "color").expect("a colour")
}

const PAGE: &str = "<style>body { margin: 0 } div { height: 50px } \
                    #a:hover { color: #f00 } #b:hover span { color: #00f }</style>\
                    <div id=a>a</div><div id=b><span id=s>b</span></div>";

#[test]
fn the_pointer_hovers_the_element_under_it_and_its_ancestors() {
    for scripts in [true, false] {
        let (mut tab, _) = scripted(PAGE, scripts);
        tab.engine.frame(200, 200);
        tab.engine.pointer_move(10.0, 10.0);
        assert_eq!(hovered(&tab), ["-", "-", "a"], "scripts {scripts}");
        assert_eq!(color(&mut tab, "a"), "rgb(255, 0, 0)");
        assert!(tab.engine.needs_frame());
        tab.engine.pointer_move(2.0, 60.0);
        assert_eq!(hovered(&tab), ["-", "-", "b", "s"]);
        assert_eq!(color(&mut tab, "a"), "rgb(0, 0, 0)");
        assert_eq!(color(&mut tab, "s"), "rgb(0, 0, 255)");
        tab.engine.pointer_leave();
        assert!(hovered(&tab).is_empty());
        assert_eq!(color(&mut tab, "s"), "rgb(0, 0, 0)");
    }
}

#[test]
fn hover_follows_the_page_moving_under_a_still_pointer() {
    let (mut tab, _) = scripted(
        "<style>body { margin: 0 } div { height: 50px }</style>\
         <div id=a>a</div><div id=b>b</div>\
         <script>setTimeout(function () { \
           document.getElementById('a').remove(); }, 0);</script>",
        true,
    );
    tab.engine.frame(200, 200);
    tab.engine.pointer_move(10.0, 10.0);
    assert_eq!(hovered(&tab), ["-", "-", "a"]);
    // The element under the pointer goes; the next frame finds what is
    // there now, and nothing gone stays hovered.
    std::thread::sleep(std::time::Duration::from_millis(2));
    tab.engine.run_tasks();
    let now = std::time::Duration::from_secs(1);
    tab.engine.frame_at(
        200,
        200,
        crate::FrameTime {
            now,
            deadline: now + std::time::Duration::from_millis(16),
        },
    );
    assert_eq!(hovered(&tab), ["-", "-", "b"]);
}

#[test]
fn the_urls_fragment_indicates_the_target() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<style>:target { color: #0f0 }</style><p id=one>1</p><p id=two>2</p>",
        )),
        _ => None,
    });
    let mut tab = Tab::new();
    super::tests::load_all(&mut tab, server.url("/page.html#one"));
    assert_eq!(color(&mut tab, "one"), "rgb(0, 255, 0)");
    tab.engine.navigate(server.url("/page.html#two"));
    assert_eq!(color(&mut tab, "one"), "rgb(0, 0, 0)");
    assert_eq!(color(&mut tab, "two"), "rgb(0, 255, 0)");
    tab.engine.go_back();
    assert_eq!(color(&mut tab, "one"), "rgb(0, 255, 0)");
}

#[test]
fn the_cursor_is_the_elements_or_the_browsers_choice() {
    let (mut tab, _) = scripted(
        "<style>body { margin: 0 } div, a, input, textarea { display: block; height: 20px; box-sizing: border-box; margin: 0 } \
         #m { cursor: move } #u { cursor: url(x.png), help }</style>\
         <div id=m>m</div><div id=u>u</div><a href=/x>link</a>\
         <input><textarea></textarea><div>plain</div><input type=checkbox>",
        false,
    );
    tab.engine.frame(200, 400);
    let at = |tab: &mut Tab, y: f32| tab.engine.cursor_at(5.0, y);
    assert_eq!(at(&mut tab, 5.0), Cursor::Move);
    assert_eq!(at(&mut tab, 25.0), Cursor::Help);
    assert_eq!(at(&mut tab, 45.0), Cursor::Pointer);
    assert_eq!(at(&mut tab, 65.0), Cursor::Text);
    assert_eq!(at(&mut tab, 85.0), Cursor::Text);
    assert_eq!(at(&mut tab, 105.0), Cursor::Default);
    // A checkbox has no text to edit.
    assert_eq!(at(&mut tab, 125.0), Cursor::Default);
    assert_eq!(at(&mut tab, 300.0), Cursor::Default);
}

#[test]
fn a_traversal_within_a_document_still_loading_moves_the_target() {
    // The image keeps the document loading through the traversal.
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<style>:target { color: #0f0 }</style><p id=one>1</p><p id=two>2</p>\
             <img src=/slow.gif>",
        )),
        "/slow.gif" => Some(net::test_server::Reply::Pieces(vec![
            (
                std::time::Duration::ZERO,
                b"HTTP/1.1 200 OK\r\nContent-Type: image/gif\r\nContent-Length: 10\r\n\r\n"
                    .to_vec(),
            ),
            (std::time::Duration::from_secs(5), b"GIF89a....".to_vec()),
        ])),
        _ => None,
    });
    let mut tab = Tab::new();
    let one = server.url("/page.html#one");
    tab.engine.navigate(one.clone());
    tab.wait_until(|engine| {
        engine.url() == Some(&one)
            && engine
                .document()
                .subtree(engine.document().root())
                .any(|node| engine.document().attr(node, "id") == Some("two"))
    });
    tab.engine.navigate(server.url("/page.html#two"));
    assert_eq!(color(&mut tab, "two"), "rgb(0, 255, 0)");
    tab.engine.go_back();
    assert_ne!(tab.engine.load_state(), &crate::LoadState::Loaded);
    assert_eq!(color(&mut tab, "one"), "rgb(0, 255, 0)");
    assert_eq!(color(&mut tab, "two"), "rgb(0, 0, 0)");
}
