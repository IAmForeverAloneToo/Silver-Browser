//! The toolbar, the status bubble and the window's title, with the
//! bundled test font: the same pixels on every machine.

use text::FontCollection;

use crate::toolbar::{FIELD_H, FIELD_X, FIELD_Y, FONT_SIZE, PADDING};
use crate::*;

fn fonts() -> FontCollection {
    FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
}

const WIDTH: u32 = 600;

/// A toolbar 600 pixels wide showing `url`.
fn toolbar(url: &str, back: bool, forward: bool, loading: bool) -> Toolbar {
    let mut toolbar = Toolbar::new(fonts());
    toolbar.update(PageState {
        url: Some(url),
        can_go_back: back,
        can_go_forward: forward,
        loading,
    });
    toolbar.frame(WIDTH, 1.0);
    toolbar
}

fn px(frame: &[u8], x: u32, y: u32) -> [u8; 3] {
    let at = ((y * WIDTH + x) * 4) as usize;
    [frame[at], frame[at + 1], frame[at + 2]]
}

/// The centre of the `n`th button.
fn button(n: f32) -> (f32, f32) {
    (4.0 + n * 36.0 + 16.0, 20.0)
}

fn click(toolbar: &mut Toolbar, (x, y): (f32, f32)) -> Option<Action> {
    toolbar.pointer_moved(x, y);
    toolbar.press(x, y, 1, false);
    toolbar.release(x, y)
}

#[test]
fn the_buttons_act_when_the_history_and_the_load_let_them() {
    let mut bar = toolbar("https://example.com/", true, false, false);
    assert_eq!(click(&mut bar, button(0.0)), Some(Action::Back));
    assert_eq!(
        click(&mut bar, button(1.0)),
        None,
        "nothing to go forward to"
    );
    assert_eq!(click(&mut bar, button(2.0)), Some(Action::Reload));
    let mut bar = toolbar("https://example.com/", false, true, true);
    assert_eq!(click(&mut bar, button(0.0)), None);
    assert_eq!(click(&mut bar, button(1.0)), Some(Action::Forward));
    assert_eq!(
        click(&mut bar, button(2.0)),
        Some(Action::Stop),
        "stop while loading"
    );
    // Pressed on one button and released on another is no click.
    bar.press(button(1.0).0, 20.0, 1, false);
    assert_eq!(bar.release(button(2.0).0, 20.0), None);
    assert_eq!(bar.part_at(button(2.0).0, 20.0), Some(Part::Reload));
    assert_eq!(bar.part_at(300.0, 20.0), Some(Part::Field));
    assert_eq!(bar.part_at(300.0, 45.0), None);
    assert_eq!(bar.cursor(300.0, 20.0), Cursor::Text);
    assert_eq!(bar.cursor(button(0.0).0, 20.0), Cursor::Arrow);
}

#[test]
fn a_button_the_history_cannot_use_is_dimmed_and_one_under_the_pointer_lights() {
    let shaft = |n: f32| (button(n).0 as u32 + 3, 19);
    let mut bar = toolbar("https://example.com/", false, true, false);
    let frame = bar.frame(WIDTH, 1.0).to_vec();
    let (x, y) = shaft(0.0);
    assert_eq!(px(&frame, x, y), [0xAA; 3], "back, dimmed");
    let (x, y) = (button(1.0).0 as u32 - 3, 19);
    assert_eq!(px(&frame, x, y), [0x3C; 3], "forward, enabled");
    assert_eq!(
        px(&frame, button(1.0).0 as u32 - 14, 7),
        [0xED; 3],
        "the bar"
    );
    bar.pointer_moved(button(1.0).0, 20.0);
    assert!(bar.needs_frame());
    let frame = bar.frame(WIDTH, 1.0).to_vec();
    assert_eq!(px(&frame, button(1.0).0 as u32 - 14, 7), [0xDA; 3], "lit");
    assert!(!bar.needs_frame());
    // A dimmed button does not light.
    bar.pointer_moved(button(0.0).0, 20.0);
    let frame = bar.frame(WIDTH, 1.0).to_vec();
    assert_eq!(px(&frame, button(0.0).0 as u32 - 14, 7), [0xED; 3]);
}

#[test]
fn a_click_focuses_the_field_with_its_text_selected_and_a_second_places_the_caret() {
    let url = "https://example.com/a";
    let mut bar = toolbar(url, false, false, false);
    assert!(!bar.is_focused());
    bar.press(300.0, 20.0, 1, false);
    bar.release(300.0, 20.0);
    assert!(bar.is_focused());
    assert_eq!(bar.selected().as_deref(), Some(url));
    // Placed at the boundary nearest the pointer.
    let stops = fonts().caret_stops(url, FONT_SIZE);
    let at = FIELD_X + PADDING + (stops[8] + stops[9]) / 2.0 - 0.5;
    bar.press(at, 20.0, 1, false);
    bar.release(at, 20.0);
    assert_eq!(bar.field().caret(), 8);
    assert_eq!(bar.selected(), None);
    // Shift with a click selects from the caret to it.
    let end = FIELD_X + PADDING + stops[19];
    bar.press(end, 20.0, 1, true);
    bar.release(end, 20.0);
    assert_eq!(bar.selected().as_deref(), Some("example.com"));
    bar.press(at, 20.0, 2, false);
    assert_eq!(bar.selected().as_deref(), Some("example"));
    bar.press(at, 20.0, 3, false);
    assert_eq!(bar.selected().as_deref(), Some(url));
}

#[test]
fn a_drag_from_the_click_that_focuses_the_field_selects_what_it_covers() {
    let url = "https://example.com/a";
    let mut bar = toolbar(url, false, false, false);
    let stops = fonts().caret_stops(url, FONT_SIZE);
    let x = |n: usize| FIELD_X + PADDING + stops[n];
    bar.press(x(8), 20.0, 1, false);
    bar.pointer_moved(x(15), 20.0);
    bar.release(x(15), 20.0);
    assert_eq!(bar.selected().as_deref(), Some("example"));
    // A pointer that trembles under the click is no drag.
    let mut bar = toolbar(url, false, false, false);
    bar.press(x(8), 20.0, 1, false);
    bar.pointer_moved(x(8) + 2.0, 21.0);
    bar.release(x(8) + 2.0, 21.0);
    assert_eq!(bar.selected().as_deref(), Some(url));
    // A field the focus left shows its text from the start at once,
    // before it is drawn again, so a drag selects what was under the
    // pointer.
    let long = format!("{url}/{}", "segment/".repeat(60));
    let mut bar = toolbar(&long, false, false, false);
    bar.focus();
    bar.command(Command::End { extend: false });
    bar.frame(WIDTH, 1.0);
    bar.blur();
    bar.press(x(8), 20.0, 1, false);
    bar.pointer_moved(x(15), 20.0);
    bar.release(x(15), 20.0);
    assert_eq!(bar.selected().as_deref(), Some("example"));
}

#[test]
fn the_selection_is_drawn_while_the_field_has_the_keyboard() {
    let text = "a          b";
    let mut bar = toolbar(text, false, false, false);
    let stops = fonts().caret_stops(text, FONT_SIZE);
    let x = (FIELD_X + PADDING + (stops[1] + stops[11]) / 2.0) as u32;
    let row = (FIELD_Y + FIELD_H / 2.0) as u32;
    assert_eq!(px(bar.frame(WIDTH, 1.0), x, row), [0xFF; 3]);
    bar.focus();
    assert_eq!(px(bar.frame(WIDTH, 1.0), x, row), [0xB4, 0xD2, 0xFA]);
    bar.blur();
    assert_eq!(px(bar.frame(WIDTH, 1.0), x, row), [0xFF; 3]);
}

#[test]
fn the_field_shows_the_page_unless_the_user_is_editing_it() {
    let mut bar = toolbar("https://a.example/", false, false, false);
    let page = |url| PageState {
        url: Some(url),
        can_go_back: false,
        can_go_forward: false,
        loading: false,
    };
    bar.update(page("https://b.example/"));
    assert_eq!(bar.text(), "https://b.example/");
    // Focused but unedited, it follows the page, all of it selected.
    bar.focus();
    bar.update(page("https://c.example/"));
    assert_eq!(bar.selected().as_deref(), Some("https://c.example/"));
    // Moving the caret, or inserting only what the field drops, is no
    // edit.
    bar.command(Command::End { extend: false });
    bar.insert("\n").unwrap();
    assert!(!bar.is_edited());
    bar.command(Command::SelectAll);
    // An edit stays, focused or not.
    bar.insert("typed").unwrap();
    assert!(bar.is_edited());
    bar.update(page("https://d.example/"));
    assert_eq!(bar.text(), "typed");
    bar.blur();
    bar.update(page("https://e.example/"));
    assert_eq!(bar.text(), "typed");
    // Escape gives the edit up, and then gives the focus back.
    bar.focus();
    assert_eq!(bar.escape(), Escape::Reverted);
    assert_eq!(bar.selected().as_deref(), Some("https://e.example/"));
    assert_eq!(bar.escape(), Escape::Unfocused);
    assert!(!bar.is_focused());
    // Typing, moving and selecting need the focus.
    bar.insert("x").unwrap();
    bar.command(Command::Backspace);
    assert_eq!(bar.text(), "https://e.example/");
}

#[test]
fn enter_shows_what_was_typed_until_its_navigation_commits_or_ends() {
    let mut bar = toolbar("https://a.example/", false, false, false);
    let page = |url, loading| PageState {
        url: Some(url),
        can_go_back: false,
        can_go_forward: false,
        loading,
    };
    bar.focus();
    bar.insert("b.example").unwrap();
    assert_eq!(bar.enter(), Some(Action::Navigate("b.example".into())));
    assert!(!bar.is_focused());
    bar.update(page("https://a.example/", true));
    assert_eq!(bar.text(), "b.example", "while it loads");
    // Escape in the field meanwhile shows the page's URL again.
    bar.focus();
    assert_eq!(bar.escape(), Escape::Reverted);
    assert_eq!(bar.text(), "https://a.example/");
    bar.blur();
    bar.update(page("https://b.example/", true));
    assert_eq!(bar.text(), "https://b.example/", "once it commits");
    // One that ends without a page shows the page's URL again.
    bar.focus();
    bar.insert("c.example").unwrap();
    bar.enter();
    bar.update(page("https://b.example/", false));
    assert_eq!(bar.text(), "https://b.example/");
    // One that never started leaves the text to edit.
    bar.focus();
    bar.insert("not an address").unwrap();
    bar.enter();
    bar.keep_editing();
    bar.update(page("https://b.example/", false));
    assert!(bar.is_focused());
    assert_eq!(bar.text(), "not an address");
}

#[test]
fn copy_cut_and_paste_go_through_the_selection() {
    let mut bar = toolbar("https://example.com/", false, false, false);
    assert_eq!(bar.selected(), None, "nothing to copy without the focus");
    bar.focus();
    assert_eq!(bar.cut().as_deref(), Some("https://example.com/"));
    assert_eq!(bar.text(), "");
    bar.insert("line one\nline two").unwrap();
    assert_eq!(bar.text(), "line oneline two");
    assert_eq!(
        bar.insert(&"x".repeat(field::MAX_LEN)),
        Err(TooLong),
        "refused whole"
    );
}

#[test]
fn the_caret_is_drawn_where_the_text_ends_and_a_long_url_scrolls_to_it() {
    let mut bar = toolbar("abc", false, false, false);
    bar.focus();
    bar.command(Command::End { extend: false });
    let frame = bar.frame(WIDTH, 1.0).to_vec();
    let stops = fonts().caret_stops("abc", FONT_SIZE);
    let caret = (FIELD_X + PADDING + stops[3]).round() as u32;
    let row = (FIELD_Y + FIELD_H / 2.0) as u32;
    assert_eq!(px(&frame, caret, row), [0x1F; 3]);
    assert_eq!(px(&frame, caret + 3, row), [0xFF; 3]);
    let top = (FIELD_Y + 6.0) as u32;
    assert_eq!(px(&frame, caret, top), [0x1F; 3]);
    bar.blur();
    assert_eq!(
        px(bar.frame(WIDTH, 1.0), caret, top),
        [0xFF; 3],
        "no caret unfocused"
    );
    // A URL wider than the field: shown from its start without the
    // focus, scrolled to keep the caret in view with it.
    let long = format!("https://example.com/{}", "segment/".repeat(60));
    let mut bar = toolbar(&long, false, false, false);
    let (x, ..) = bar.caret_area();
    assert!(x > WIDTH as f32, "unfocused, the end is out of view: {x}");
    bar.focus();
    bar.command(Command::End { extend: false });
    bar.frame(WIDTH, 1.0);
    let (x, ..) = bar.caret_area();
    assert!(
        x <= WIDTH as f32 - 8.0 - PADDING + 0.5 && x > FIELD_X,
        "{x}"
    );
    // A click in the scrolled text places the caret under the pointer.
    bar.press(300.0, 20.0, 1, false);
    let (x, ..) = bar.caret_area();
    assert!((x - 300.0).abs() < 6.0, "{x}");
    bar.command(Command::Home { extend: false });
    bar.frame(WIDTH, 1.0);
    let (x, ..) = bar.caret_area();
    assert_eq!(x, FIELD_X + PADDING);
    // A caret moved past the field's end scrolls the text just enough
    // to stand at that end.
    for _ in 0..80 {
        bar.command(Command::Right { extend: false });
    }
    bar.frame(WIDTH, 1.0);
    let (x, ..) = bar.caret_area();
    assert!((x - (WIDTH as f32 - 8.0 - PADDING)).abs() < 0.01, "{x}");
}

/// The text the field draws, and the x it is drawn at.
fn drawn_text(bar: &mut Toolbar) -> (String, f32) {
    bar.display_list()
        .items
        .into_iter()
        .find_map(|item| match item {
            paint::DisplayItem::Text { text, x, .. } => Some((text, x)),
            _ => None,
        })
        .expect("the field's text is drawn")
}

#[test]
fn only_the_characters_in_view_are_drawn() {
    let long = format!("https://example.com/{}", "segment/".repeat(60));
    let stops = fonts().caret_stops(&long, FONT_SIZE);
    let inner_w = WIDTH as f32 - FIELD_X - 8.0 - 2.0 * PADDING;
    let mut bar = toolbar(&long, false, false, false);
    let (text, x) = drawn_text(&mut bar);
    assert!(long.starts_with(&text));
    assert_eq!(x, FIELD_X + PADDING);
    // Up to the first boundary at or past the field's end.
    let n = text.len();
    assert!(stops[n] >= inner_w && stops[n - 1] < inner_w, "{n}");
    bar.focus();
    bar.command(Command::End { extend: false });
    let (text, x) = drawn_text(&mut bar);
    assert!(long.ends_with(&text));
    // From the last boundary at or before the field's start.
    let first = long.len() - text.len();
    let scroll = stops[long.len()] - inner_w;
    assert!(
        stops[first] <= scroll && stops[first + 1] > scroll,
        "{first}"
    );
    assert!((x - (FIELD_X + PADDING + stops[first] - scroll)).abs() < 0.01);
}

#[test]
fn the_window_title_is_the_pages_and_the_browsers_name() {
    assert_eq!(
        window_title("Example Domain", Some("https://example.com/")),
        "Example Domain — Silver Browser"
    );
    assert_eq!(
        window_title("", Some("https://example.com/")),
        "https://example.com/ — Silver Browser"
    );
    assert_eq!(window_title("", None), "Silver Browser");
    // A title cannot reorder the browser's name, nor run on.
    assert_eq!(
        window_title("\u{202E}evil\u{2066}\u{200F} page\u{7}", None),
        "evil page — Silver Browser"
    );
    let long = window_title(&"t".repeat(5000), None);
    assert_eq!(long.chars().filter(|&c| c == 't').count(), 1024);
    // What is dropped and the spaces at the ends do not count.
    let padded = format!("{}{}", "\u{202E} ".repeat(10), "t".repeat(5000));
    assert_eq!(window_title(&padded, None), long);
}

#[test]
fn the_status_bubble_fits_its_text_and_keeps_its_start() {
    let mut bubble = StatusBubble::new(fonts());
    let url = "https://example.com/a/rather/long/path/that/goes/on";
    assert_eq!(bubble.fit(url, 2000.0), url);
    let fitted = bubble.fit(url, 120.0);
    assert!(
        fitted.starts_with("https://") && fitted.ends_with('…'),
        "{fitted}"
    );
    assert!(fonts().measure(&fitted, 13.0) <= 120.0 - 12.0 + 0.01);
    let (width, pixels) = bubble.frame(url, 120, 1.0).expect("a bubble");
    assert!(width <= 120);
    assert_eq!(pixels.len(), (width * status::HEIGHT * 4) as usize);
    assert!(bubble.frame("", 400, 1.0).is_none());
}
