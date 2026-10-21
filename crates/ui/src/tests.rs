//! The toolbar, the status bubble, the tab strip and the window's
//! title, with the bundled test font: the same pixels on every machine.

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

#[test]
fn the_prompt_bar_draws_its_text_and_its_buttons() {
    let mut bar = PromptBar::new(fonts());
    let text = "https://a.test wants to connect to devices on your local network";
    let pixels = bar.frame(text, WIDTH, 1.0).to_vec();
    assert_eq!(pixels.len(), (WIDTH * prompt::HEIGHT * 4) as usize);
    // Block at the right, Allow before it, nothing elsewhere.
    let (w, middle) = (WIDTH as f32, prompt::HEIGHT as f32 / 2.0);
    assert_eq!(
        PromptBar::choice_at(w, w - 20.0, middle),
        Some(Choice::Block)
    );
    assert_eq!(
        PromptBar::choice_at(w, w - 100.0, middle),
        Some(Choice::Allow)
    );
    assert_eq!(PromptBar::choice_at(w, 20.0, middle), None);
    assert_eq!(PromptBar::choice_at(w, w - 20.0, 1.0), None);
    // The text is drawn at the left, the labels on the buttons.
    let dark = |x0: u32, x1: u32| {
        (0..prompt::HEIGHT).any(|y| (x0..x1).any(|x| pixels[((y * WIDTH + x) * 4) as usize] < 0x80))
    };
    assert!(dark(10, 60), "the text");
    assert!(dark(WIDTH - 60, WIDTH - 30), "a label");
    assert!(!dark(WIDTH - 9, WIDTH), "the margin");
    // Twice the rows at twice the scale.
    let doubled = bar.frame(text, WIDTH * 2, 2.0).len();
    assert_eq!(doubled, (WIDTH * 2 * prompt::HEIGHT * 2 * 4) as usize);
}

/// A strip `width` pixels wide of `count` tabs, `shown` the one shown.
fn strip(width: u32, count: usize, shown: usize) -> TabStrip {
    let mut strip = TabStrip::new(fonts());
    let tabs = (0..count)
        .map(|n| TabItem {
            title: format!("Page {n}"),
            loading: false,
        })
        .collect();
    strip.update(tabs, shown);
    strip.frame(width, 1.0);
    strip
}

/// The middle of tab `n`'s title, and of its close button.
fn tab_at(strip: &TabStrip, n: usize) -> (f32, f32) {
    (6.0 + strip.tab_width() * (n as f32 + 0.3), 18.0)
}

fn close_at(strip: &TabStrip, n: usize) -> (f32, f32) {
    (6.0 + strip.tab_width() * (n as f32 + 1.0) - 18.0, 18.0)
}

fn strip_click(strip: &mut TabStrip, (x, y): (f32, f32), middle: bool) -> Vec<StripAction> {
    strip.pointer_moved(x, y);
    let mut done: Vec<StripAction> = strip.press(x, y, middle).into_iter().collect();
    done.extend(strip.release(x, y));
    done
}

#[test]
fn tabs_are_their_width_while_they_fit_and_narrower_as_more_come() {
    assert_eq!(strip(800, 3, 0).tab_width(), tabs::TAB_WIDTH);
    let crowded = strip(800, 10, 0).tab_width();
    assert!(
        crowded < tabs::TAB_WIDTH && crowded > tabs::MIN_TAB_WIDTH,
        "{crowded}"
    );
    assert_eq!(strip(800, 100, 0).tab_width(), tabs::MIN_TAB_WIDTH);
}

#[test]
fn a_tab_pressed_is_shown_and_its_close_button_closes_it() {
    let mut strip = strip(800, 3, 0);
    assert_eq!(
        {
            let at = tab_at(&strip, 2);
            strip_click(&mut strip, at, false)
        },
        [StripAction::Select(2)]
    );
    strip.update(
        (0..3)
            .map(|n| TabItem {
                title: format!("Page {n}"),
                loading: false,
            })
            .collect(),
        2,
    );
    let close = close_at(&strip, 2);
    assert_eq!(strip.part_at(close.0, close.1), Some(StripPart::Close(2)));
    assert_eq!(
        strip_click(&mut strip, close, false),
        [StripAction::Close(2)]
    );
    // A tab neither shown nor under the pointer has no close button.
    strip.pointer_left();
    let other = close_at(&strip, 0);
    assert_eq!(strip.part_at(other.0, other.1), Some(StripPart::Tab(0)));
}

#[test]
fn the_middle_button_closes_a_tab_and_the_new_tab_button_opens_one() {
    let mut strip = strip(800, 3, 0);
    assert_eq!(
        {
            let at = tab_at(&strip, 1);
            strip_click(&mut strip, at, true)
        },
        [StripAction::Close(1)]
    );
    let new = (6.0 + 3.0 * tabs::TAB_WIDTH + 3.0 + 14.0, 18.0);
    assert_eq!(strip.part_at(new.0, new.1), Some(StripPart::New));
    assert_eq!(strip_click(&mut strip, new, false), [StripAction::New]);
    // A close button pressed and released elsewhere closes nothing.
    let mut strip = self::strip(800, 3, 0);
    let close = close_at(&strip, 0);
    strip.pointer_moved(close.0, close.1);
    assert_eq!(strip.press(close.0, close.1, false), None);
    assert_eq!(strip.release(400.0, 18.0), None);
}

#[test]
fn a_tab_dragged_moves_among_the_others() {
    let mut strip = strip(800, 3, 0);
    let (x, y) = tab_at(&strip, 0);
    strip.pointer_moved(x, y);
    assert_eq!(strip.press(x, y, false), Some(StripAction::Select(0)));
    assert_eq!(
        strip.pointer_moved(x + 3.0, y),
        None,
        "a tremble is no drag"
    );
    assert_eq!(
        strip.pointer_moved(x + tabs::TAB_WIDTH * 0.7, y),
        Some(StripAction::Move { from: 0, to: 1 })
    );
    assert_eq!(
        strip.pointer_moved(x + tabs::TAB_WIDTH * 1.7, y),
        Some(StripAction::Move { from: 1, to: 2 })
    );
    assert_eq!(strip.release(x + tabs::TAB_WIDTH * 1.7, y), None);
}

#[test]
fn many_tabs_scroll_and_the_one_shown_stays_in_sight() {
    let mut strip = strip(400, 40, 39);
    let w = strip.tab_width();
    assert_eq!(w, tabs::MIN_TAB_WIDTH);
    // The last tab, shown, is in sight at the strip's right end.
    let room = 400.0 - 6.0 * 3.0 - 28.0;
    assert_eq!(
        strip.part_at(6.0 + room - w / 2.0, 18.0),
        Some(StripPart::Tab(39))
    );
    strip.wheel(-10_000.0);
    assert_eq!(strip.part_at(6.0 + w / 2.0, 18.0), Some(StripPart::Tab(0)));
    // Scrolled to the far end, a tab before the view shown is brought
    // back into it.
    strip.wheel(10_000.0);
    let tabs = (0..40)
        .map(|n| TabItem {
            title: format!("Page {n}"),
            loading: false,
        })
        .collect();
    strip.update(tabs, 3);
    assert_eq!(strip.part_at(6.0 + w / 2.0, 18.0), Some(StripPart::Tab(3)));
    // Tabs that fit do not scroll.
    let mut few = self::strip(800, 2, 0);
    few.wheel(500.0);
    assert_eq!(few.part_at(6.0 + 10.0, 18.0), Some(StripPart::Tab(0)));
}

#[test]
fn the_tab_shown_is_joined_to_the_toolbar_and_a_load_is_marked() {
    let mut strip = TabStrip::new(fonts());
    strip.update(
        vec![
            TabItem {
                title: "Shown".into(),
                loading: false,
            },
            TabItem {
                title: "Loading".into(),
                loading: true,
            },
        ],
        0,
    );
    let frame = strip.frame(WIDTH, 1.0).to_vec();
    let at = |x: u32, y: u32| {
        let i = ((y * WIDTH + x) * 4) as usize;
        [frame[i], frame[i + 1], frame[i + 2]]
    };
    let bottom = tabs::HEIGHT - 1;
    // Under the tab shown, the toolbar's colour; under the other, the
    // strip's edge.
    assert_eq!(at(100, bottom), [0xED, 0xED, 0xED]);
    assert_eq!(at(6 + 220 + 100, bottom), [0xB4, 0xB4, 0xB4]);
    // The other tab's mark: the dot's blue before its title.
    let blue = (6 + 220 + 10..6 + 220 + 22).any(|x| at(x, 18) == [0x38, 0x74, 0xD8]);
    assert!(blue, "the loading mark");
    // Each title is drawn.
    let dark = |from: u32| (from + 10..from + 120).any(|x| (8..30).any(|y| at(x, y)[0] < 0x80));
    assert!(dark(6) && dark(6 + 220));
}
