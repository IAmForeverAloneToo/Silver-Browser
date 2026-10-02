//! The session driven as the window drives it, with an engine over the
//! bundled font, a scripted server on 127.0.0.1 and a clipboard of the
//! tests' own: focusing the field, typing an address, Enter, the load,
//! the title, the links, back and forward, the clipboard, the input
//! method, and the frame put together.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use engine::{Engine, FrameTime, LoadState, Url};
use net::test_server::{Reply, TestServer};

use super::*;
use crate::keys::{Key, Modifiers, Platform};

/// Long enough for anything a test waits for on a loaded machine.
const WAIT: Duration = Duration::from_secs(30);
const WIDTH: u32 = 800;
const HEIGHT: u32 = 600;

/// A clipboard of the tests' own, which can be made unreachable.
#[derive(Clone, Default)]
struct Board(Rc<RefCell<(String, bool)>>);

impl Board {
    fn text(&self) -> String {
        self.0.borrow().0.clone()
    }

    fn put(&self, text: &str) {
        self.0.borrow_mut().0 = text.to_owned();
    }

    fn break_it(&self) {
        self.0.borrow_mut().1 = true;
    }
}

impl Clipboard for Board {
    fn get(&mut self) -> Result<String, String> {
        let board = self.0.borrow();
        if board.1 {
            Err("no display".to_owned())
        } else {
            Ok(board.0.clone())
        }
    }

    fn set(&mut self, text: &str) -> Result<(), String> {
        let mut board = self.0.borrow_mut();
        if board.1 {
            return Err("no display".to_owned());
        }
        board.0 = text.to_owned();
        Ok(())
    }
}

/// A window: the session, the channel its engine's waker sends on, the
/// clipboard, and the shell's clock.
struct Window {
    session: Session,
    woken: Receiver<()>,
    board: Board,
    clock: Duration,
}

impl Window {
    fn new(start: Option<Url>, platform: Platform) -> Window {
        let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses");
        let mut engine = Engine::with_fonts(fonts);
        let (wake, woken) = channel();
        engine.set_waker(move || {
            let _ = wake.send(());
        });
        let board = Board::default();
        let mut session = Session::new(engine, Box::new(board.clone()), platform, start);
        session.resized(WIDTH, HEIGHT);
        let mut window = Window {
            session,
            woken,
            board,
            clock: Duration::from_secs(1),
        };
        window.frame();
        window
    }

    /// Takes results as the waker says they come, as the shell does,
    /// until `done` holds; a frame is drawn after each, as the window
    /// asks for one.
    fn wait_until(&mut self, done: impl Fn(&Session) -> bool) {
        let deadline = Instant::now() + WAIT;
        while !done(&self.session) {
            let left = deadline
                .checked_duration_since(Instant::now())
                .expect("the session gets there in time");
            self.woken
                .recv_timeout(left)
                .expect("a fetch finishes in time");
            self.session.network();
            self.frame();
        }
    }

    /// Waits for the page under way to be shown, all there.
    fn settle(&mut self) {
        self.wait_until(|session| *session.engine().load_state() != LoadState::Loading);
    }

    fn frame(&mut self) -> Vec<u32> {
        let mut out = vec![0; (WIDTH * HEIGHT) as usize];
        self.session
            .draw(WIDTH, HEIGHT, FrameTime::at(self.clock), &mut out);
        out
    }

    fn mods(&mut self, mods: Modifiers) {
        self.session.modifiers(mods);
    }

    /// A key pressed with `mods` held, typing nothing.
    fn press_key(&mut self, key: Key, mods: Modifiers) {
        self.mods(mods);
        self.session.key(key, None);
        self.mods(Modifiers::default());
    }

    /// A character key pressed with `mods` held: the text it would type
    /// comes with it, as winit's does, Ctrl or not.
    fn chord(&mut self, c: &str, mods: Modifiers) {
        self.mods(mods);
        self.session.key(Key::Char(c.to_owned()), Some(c));
        self.mods(Modifiers::default());
    }

    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            let key = if c == ' ' {
                Key::Space
            } else {
                Key::Char(c.to_string())
            };
            self.session.key(key, Some(&c.to_string()));
        }
    }

    /// A press and a release of the left button at `(x, y)`, a second
    /// after the last.
    fn click(&mut self, x: f32, y: f32) {
        self.clock += Duration::from_secs(1);
        self.press_at(x, y, self.clock);
        self.session.release(Button::Left);
    }

    fn press_at(&mut self, x: f32, y: f32, at: Duration) {
        self.session.pointer_moved(x, y);
        self.session.press(Button::Left, at);
    }

    fn text(&self) -> &str {
        self.session.toolbar().text()
    }

    fn selected(&self) -> Option<String> {
        self.session.toolbar().selected()
    }
}

const NONE: Modifiers = Modifiers {
    ctrl: false,
    shift: false,
    alt: false,
    logo: false,
};
const CTRL: Modifiers = Modifiers { ctrl: true, ..NONE };
const SHIFT: Modifiers = Modifiers {
    shift: true,
    ..NONE
};
const ALT: Modifiers = Modifiers { alt: true, ..NONE };
const CMD: Modifiers = Modifiers { logo: true, ..NONE };

/// A server answering each path with what `route` gives, and 404 for
/// the paths it gives nothing for.
fn serve(route: impl Fn(&str) -> Option<Reply> + Send + Sync + 'static) -> TestServer {
    TestServer::plain(move |request| {
        route(&request.target).unwrap_or_else(|| {
            Reply::with_fields(404, &[("Content-Type", "text/plain")], b"not here")
        })
    })
}

fn html(body: &str) -> Option<Reply> {
    Some(Reply::ok("text/html", body.as_bytes()))
}

/// Two pages, the first with a link to the second as a block across
/// the top of the page: at (50, 60) in the window, below the toolbar.
fn two_pages() -> TestServer {
    serve(|path| match path {
        "/" => html(
            "<title>First</title><body style='margin:0'>\
             <a href=/second style='display:block;height:100px'>next</a>\
             <p>the first page</p>",
        ),
        "/second" => html("<title>Second</title><p>the second page"),
        _ => None,
    })
}

#[test]
fn with_nothing_asked_for_about_silver_shows_with_the_field_focused() {
    let mut window = Window::new(None, Platform::Other);
    window.settle();
    assert_eq!(window.session.focus(), Focus::Field);
    assert_eq!(window.selected().as_deref(), Some("about:silver"));
    assert_eq!(window.session.title(), "Silver Browser — Silver Browser");
    assert!(
        window.session.ime_area().is_some(),
        "the input method is on"
    );
    // Asked for a page, the page has the keyboard.
    let server = two_pages();
    let window = Window::new(Some(server.url("/")), Platform::Other);
    assert_eq!(window.session.focus(), Focus::Page);
}

#[test]
fn an_address_typed_and_entered_is_loaded_and_its_links_followed_back_and_forth() {
    let server = two_pages();
    let mut window = Window::new(None, Platform::Other);
    window.settle();
    // Typing replaces the selected text.
    let typed = format!("127.0.0.1:{}", server.port());
    window.type_text(&typed);
    assert_eq!(window.text(), typed);
    window.press_key(Key::Enter, NONE);
    assert_eq!(window.session.focus(), Focus::Page);
    assert_eq!(window.text(), typed, "what was typed, while it loads");
    window.settle();
    let first = server.url("/");
    assert_eq!(window.text(), first.as_str());
    assert_eq!(window.session.title(), "First — Silver Browser");
    // The link: where it goes in the bubble, a hand over it.
    window.session.pointer_moved(50.0, 60.0);
    let second = server.url("/second");
    assert_eq!(window.session.bubble_text(), Some(second.as_str()));
    assert_eq!(window.session.shape(), Shape::Hand);
    window.session.pointer_moved(50.0, 400.0);
    assert_eq!(window.session.bubble_text(), None);
    assert_eq!(window.session.shape(), Shape::Arrow);
    window.session.pointer_moved(300.0, 20.0);
    assert_eq!(window.session.shape(), Shape::Text, "over the field");
    window.click(50.0, 60.0);
    window.settle();
    assert_eq!(window.session.title(), "Second — Silver Browser");
    assert_eq!(window.text(), second.as_str());
    // Back with the keys, forward with the mouse's button, back with
    // the toolbar's.
    window.press_key(Key::Left, ALT);
    window.settle();
    assert_eq!(window.session.title(), "First — Silver Browser");
    assert_eq!(window.text(), first.as_str());
    window.session.press(Button::Forward, window.clock);
    window.settle();
    assert_eq!(window.text(), second.as_str());
    window.click(20.0, 20.0);
    window.settle();
    assert_eq!(window.text(), first.as_str());
    window.session.press(Button::Back, window.clock);
    window.settle();
    assert_eq!(window.text(), "about:silver");
    assert!(!window.session.engine().can_go_back());
}

#[test]
fn the_bubble_says_where_a_link_goes_before_any_notice() {
    let server = serve(|path| match path {
        "/" => html(
            "<body style='margin:0'>\
             <a href=mailto:someone@example.com style='display:block;height:100px'>mail</a>\
             <a href=/next style='display:block;height:100px'>next</a>",
        ),
        _ => None,
    });
    let mut window = Window::new(Some(server.url("/")), Platform::Other);
    window.settle();
    // A link the browser cannot follow: the notice shows once the
    // pointer leaves it.
    window.click(50.0, 60.0);
    assert_eq!(
        window.session.bubble_text(),
        Some("mailto:someone@example.com")
    );
    window.session.pointer_moved(50.0, 400.0);
    let notice = window.session.bubble_text().expect("a notice").to_owned();
    assert!(notice.contains("mailto"), "{notice}");
    // A press on a link released off it follows nothing.
    window.press_at(50.0, 160.0, window.clock + Duration::from_secs(5));
    window.session.pointer_moved(50.0, 400.0);
    window.session.release(Button::Left);
    assert_eq!(*window.session.engine().load_state(), LoadState::Loaded);
    // A drag from the field over the page finds no link.
    window.chord("l", CTRL);
    window.press_at(300.0, 20.0, window.clock + Duration::from_secs(10));
    window.session.pointer_moved(50.0, 160.0);
    assert_eq!(window.session.bubble_text(), None);
    assert_eq!(window.session.shape(), Shape::Arrow);
    window.session.release(Button::Left);
    // The pointer leaving the window takes the bubble with it.
    window.session.pointer_moved(50.0, 160.0);
    window.frame();
    window.session.pointer_left();
    assert_eq!(window.session.bubble_text(), None);
    assert!(window.session.needs_frame());
}

#[test]
fn what_is_not_an_address_stays_in_the_field_with_a_notice() {
    let mut window = Window::new(None, Platform::Other);
    window.settle();
    window.type_text("two words");
    window.press_key(Key::Enter, NONE);
    assert_eq!(window.session.focus(), Focus::Field);
    assert_eq!(window.text(), "two words");
    let notice = window.session.bubble_text().expect("a notice").to_owned();
    assert!(notice.starts_with("Not an address"), "{notice}");
    // Shown until the next input.
    window.frame();
    assert!(!window.session.needs_frame());
    window.press_key(Key::End, NONE);
    assert_eq!(window.session.bubble_text(), None);
    assert!(window.session.needs_frame(), "the bubble goes");
    window.chord("a", CTRL);
    window.type_text("mailto:someone@example.com");
    window.press_key(Key::Enter, NONE);
    assert!(window.session.bubble_text().unwrap().contains("mailto"));
    assert_eq!(window.text(), "mailto:someone@example.com");
    assert_eq!(
        window.session.engine().url().unwrap().as_str(),
        "about:silver"
    );
}

/// A gate a server's answer waits at until it is opened.
#[derive(Clone, Default)]
struct Gate(Arc<(Mutex<bool>, Condvar)>);

impl Gate {
    fn wait(&self) {
        let (open, opened) = &*self.0;
        let open = open.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = opened.wait_timeout_while(open, WAIT, |open| !*open);
    }

    fn open(&self) {
        let (open, opened) = &*self.0;
        *open.lock().unwrap_or_else(PoisonError::into_inner) = true;
        opened.notify_all();
    }
}

#[test]
fn escape_gives_up_an_edit_then_the_focus_and_on_the_page_stops_a_load() {
    let gate = Gate::default();
    let held = gate.clone();
    let server = serve(move |path| match path {
        "/" => html("<title>Here</title>here"),
        "/slow" => {
            held.wait();
            html("slow")
        }
        _ => None,
    });
    let here = server.url("/");
    let mut window = Window::new(Some(here.clone()), Platform::Other);
    window.settle();
    window.chord("l", CTRL);
    assert_eq!(window.session.focus(), Focus::Field);
    window.type_text("abc");
    window.press_key(Key::Escape, NONE);
    assert_eq!(window.selected().as_deref(), Some(here.as_str()));
    assert_eq!(window.session.focus(), Focus::Field);
    window.press_key(Key::Escape, NONE);
    assert_eq!(window.session.focus(), Focus::Page);
    assert!(
        window.session.ime_area().is_none(),
        "the input method is off"
    );
    // F6 focuses too; a load under way, Escape on the page stops it.
    window.press_key(Key::F6, NONE);
    window.type_text(server.url("/slow").as_str());
    window.press_key(Key::Enter, NONE);
    assert_eq!(*window.session.engine().load_state(), LoadState::Loading);
    window.press_key(Key::Escape, NONE);
    assert_eq!(*window.session.engine().load_state(), LoadState::Loaded);
    assert_eq!(window.text(), here.as_str(), "the field shows the page");
    gate.open();
}

#[test]
fn escape_on_a_page_all_there_stops_nothing() {
    let gate = Gate::default();
    let held = gate.clone();
    let server = serve(move |path| match path {
        "/" => html("<title>Here</title>here"),
        "/favicon.ico" => {
            held.wait();
            Some(Reply::ok("image/png", b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"))
        }
        _ => None,
    });
    let mut window = Window::new(Some(server.url("/")), Platform::Other);
    window.settle();
    assert_eq!(*window.session.engine().load_state(), LoadState::Loaded);
    // The icon is on its way, which is no load of the page's.
    window.press_key(Key::Escape, NONE);
    gate.open();
    window.wait_until(|session| session.engine().icon().is_some());
}

#[test]
fn copy_cut_and_paste_go_through_the_clipboard() {
    let server = two_pages();
    let first = server.url("/");
    let mut window = Window::new(Some(first.clone()), Platform::Other);
    window.settle();
    // The page has nothing to copy.
    window.board.put("before");
    window.chord("c", CTRL);
    assert_eq!(window.board.text(), "before");
    window.press_key(Key::Char("d".into()), ALT);
    assert_eq!(window.selected().as_deref(), Some(first.as_str()));
    window.chord("c", CTRL);
    assert_eq!(window.board.text(), first.as_str());
    window.chord("x", CTRL);
    assert_eq!(window.text(), "");
    window.board.put("example.org");
    window.chord("v", CTRL);
    window.press_key(Key::Insert, SHIFT);
    assert_eq!(window.text(), "example.orgexample.org");
    window.chord("a", CTRL);
    window.press_key(Key::Insert, CTRL);
    assert_eq!(window.board.text(), "example.orgexample.org");
    window.press_key(Key::Delete, SHIFT);
    assert_eq!(window.text(), "");
    // Text typed with Ctrl held is a shortcut's, not the field's.
    window.chord("q", CTRL);
    assert_eq!(window.text(), "");
    // A clipboard that cannot be reached is said to be, and a cut
    // keeps its text.
    window.type_text("kept");
    window.chord("a", CTRL);
    window.board.break_it();
    window.chord("x", CTRL);
    assert_eq!(window.text(), "kept");
    window.chord("v", CTRL);
    assert_eq!(
        window.session.bubble_text(),
        Some("The clipboard could not be reached: no display")
    );
    assert_eq!(window.text(), "kept");
}

#[test]
fn a_paste_too_long_for_the_field_is_refused_whole_with_a_notice() {
    let mut window = Window::new(None, Platform::Other);
    window.settle();
    window.board.put(&"x".repeat(ui::field::MAX_LEN + 1));
    window.chord("v", CTRL);
    assert_eq!(window.text(), "about:silver");
    assert!(window.session.bubble_text().unwrap().contains("at most"));
}

#[test]
fn on_macos_cmd_makes_the_shortcuts() {
    let server = two_pages();
    let mut window = Window::new(Some(server.url("/")), Platform::Mac);
    window.settle();
    window.chord("l", CTRL);
    assert_eq!(window.session.focus(), Focus::Page, "Ctrl is not Cmd");
    window.chord("l", CMD);
    assert_eq!(window.session.focus(), Focus::Field);
    // Cmd's letters type nothing.
    window.press_key(Key::End, NONE);
    window.chord("q", CMD);
    assert_eq!(window.text(), server.url("/").as_str());
    window.chord("a", CMD);
    window.type_text(server.url("/second").as_str());
    window.press_key(Key::Enter, NONE);
    window.settle();
    window.chord("[", CMD);
    window.settle();
    assert_eq!(window.text(), server.url("/").as_str());
    window.chord("]", CMD);
    window.settle();
    assert_eq!(window.text(), server.url("/second").as_str());
}

#[test]
fn presses_in_quick_succession_select_a_word_then_all() {
    let server = two_pages();
    let mut window = Window::new(Some(server.url("/path")), Platform::Other);
    window.settle();
    window.click(300.0, 20.0);
    assert_eq!(window.session.focus(), Focus::Field);
    window.press_key(Key::End, NONE);
    let (end, ..) = window.session.ime_area().expect("the caret");
    let at = end - 3.0;
    let t = window.clock + Duration::from_secs(5);
    window.press_at(at, 20.0, t);
    window.session.release(Button::Left);
    assert_eq!(window.selected(), None, "a click places the caret");
    window.press_at(at, 20.0, t + Duration::from_millis(200));
    window.session.release(Button::Left);
    assert_eq!(window.selected().as_deref(), Some("path"));
    window.press_at(at, 20.0, t + Duration::from_millis(400));
    window.session.release(Button::Left);
    assert_eq!(
        window.selected().as_deref(),
        Some(server.url("/path").as_str())
    );
    // Too late, or too far, is a click again.
    window.press_at(at, 20.0, t + Duration::from_millis(1000));
    window.session.release(Button::Left);
    assert_eq!(window.selected(), None);
    window.press_at(at + 20.0, 20.0, t + Duration::from_millis(1100));
    window.session.release(Button::Left);
    assert_eq!(window.selected(), None);
    // Shift with a press selects from the caret.
    window.mods(SHIFT);
    window.press_at(at - 30.0, 20.0, t + Duration::from_millis(3000));
    window.session.release(Button::Left);
    window.mods(NONE);
    assert!(
        window
            .selected()
            .is_some_and(|selected| !selected.is_empty())
    );
}

#[test]
fn a_press_on_the_page_takes_the_keyboard_and_the_keys_scroll_it() {
    let server = serve(|path| match path {
        "/" => html(&format!(
            "<body style='margin:0'><a href=/x style='display:block;height:100px'>x</a>{}",
            "<p style='height:200px'>a tall paragraph".repeat(20)
        )),
        _ => None,
    });
    let mut window = Window::new(None, Platform::Other);
    window.settle();
    window.chord("a", CTRL);
    window.type_text(server.url("/").as_str());
    window.press_key(Key::Enter, NONE);
    window.settle();
    window.chord("l", CTRL);
    window.click(400.0, 300.0);
    assert_eq!(window.session.focus(), Focus::Page);
    window.press_key(Key::Down, NONE);
    assert_eq!(window.session.engine().scroll_target(), LINE_PX);
    // A page is the page's height, below the toolbar, less a line.
    window.press_key(Key::Space, NONE);
    let page = (HEIGHT - ui::HEIGHT) as f32 - LINE_PX;
    assert_eq!(window.session.engine().scroll_target(), LINE_PX + page);
    window.press_key(Key::Home, NONE);
    assert_eq!(window.session.engine().scroll_target(), 0.0);
    // The wheel scrolls the page under the pointer, not over the
    // toolbar.
    window.session.pointer_moved(400.0, 20.0);
    window.session.wheel(LINE_PX);
    assert_eq!(window.session.engine().scroll_target(), 0.0);
    window.session.pointer_moved(400.0, 300.0);
    window.session.wheel(LINE_PX);
    assert_eq!(window.session.engine().scroll_target(), LINE_PX);
    window.press_key(Key::Home, NONE);
    window.frame();
    // The scrollbar is over the link at the page's right edge: no link
    // there. Its thumb, at the top, follows a drag.
    let bar_x = (WIDTH - 4) as f32;
    window.session.pointer_moved(bar_x, 60.0);
    assert_eq!(window.session.bubble_text(), None);
    window.session.pointer_moved(50.0, 60.0);
    assert!(window.session.bubble_text().is_some(), "the link");
    window.press_at(bar_x, ui::HEIGHT as f32 + 5.0, window.clock);
    window
        .session
        .pointer_moved(bar_x, ui::HEIGHT as f32 + 105.0);
    window.session.release(Button::Left);
    assert!(window.session.engine().scroll_target() > 0.0, "dragged");
    // In the field, the space is typed.
    let scrolled = window.session.engine().scroll_target();
    window.chord("l", CTRL);
    window.press_key(Key::End, NONE);
    window.type_text(" ");
    assert!(window.text().ends_with(' '));
    assert_eq!(window.session.engine().scroll_target(), scrolled);
}

#[test]
fn an_input_method_commits_into_the_field_and_has_the_keys_while_it_composes() {
    let mut window = Window::new(None, Platform::Other);
    window.settle();
    window.session.preedit("にほ");
    window.type_text("a");
    assert_eq!(window.text(), "about:silver", "the input method's key");
    window.session.commit("日本");
    assert_eq!(window.text(), "日本");
    window.type_text("x");
    assert_eq!(window.text(), "日本x");
}

#[test]
fn the_toolbar_is_drawn_above_the_page_and_the_bubble_over_it() {
    let server = serve(|path| match path {
        "/" => html(&format!(
            "<style>html {{ background-color: #00ff00 }}</style><body style='margin:0'>\
             <a href=/next?{} style='display:block;height:100px'>next</a>\
             <div style='position:fixed;bottom:0;left:0;right:0;height:10px;\
             background-color:#ff0000'></div>",
            "q".repeat(400)
        )),
        _ => None,
    });
    let mut window = Window::new(Some(server.url("/")), Platform::Other);
    window.settle();
    let frame = window.frame();
    let at = |frame: &[u32], x: u32, y: u32| frame[(y * WIDTH + x) as usize];
    assert_eq!(at(&frame, 0, 0), 0xEDEDED, "the toolbar");
    assert_eq!(at(&frame, 400, ui::HEIGHT - 1), 0xC9C9C9, "its edge");
    assert_eq!(at(&frame, 400, 300), 0x00FF00, "the page");
    assert_eq!(at(&frame, 2, HEIGHT - 30), 0x00FF00, "no bubble");
    assert_eq!(
        at(&frame, WIDTH - 20, HEIGHT - 2),
        0xFF0000,
        "the page's bottom"
    );
    assert!(!window.session.needs_frame());
    window.session.pointer_moved(50.0, 60.0);
    assert!(window.session.needs_frame(), "the bubble comes");
    let frame = window.frame();
    assert_eq!(at(&frame, 2, HEIGHT - 5), 0xF2F2F2, "the bubble");
    assert_eq!(
        at(&frame, WIDTH / 2 - 16, HEIGHT - 5),
        0xF2F2F2,
        "cut short"
    );
    assert_eq!(
        at(&frame, WIDTH / 2 + 2, HEIGHT - 5),
        0xFF0000,
        "half the width at most"
    );
    assert!(!window.session.needs_frame());
    window.session.pointer_left();
    let frame = window.frame();
    assert_eq!(at(&frame, 2, HEIGHT - 5), 0xFF0000);
}

#[test]
fn a_failed_start_is_reported() {
    let server = two_pages();
    let gone = server.url("/");
    drop(server);
    let mut window = Window::new(Some(gone.clone()), Platform::Other);
    window.wait_until(|session| matches!(session.engine().load_state(), LoadState::Failed(_)));
    let reports = window.session.take_reports();
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert!(reports[0].starts_with(gone.as_str()), "{reports:?}");
    assert!(window.session.take_reports().is_empty());
    // The error page shows under the URL, with its title.
    assert_eq!(window.text(), gone.as_str());
    assert!(
        window.session.title().starts_with("Can't reach"),
        "{}",
        window.session.title()
    );
}
