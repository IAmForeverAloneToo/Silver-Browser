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

/// The tab strip's height, and the strip's and the toolbar's together:
/// the page's top (M37).
const STRIP_H: u32 = ui::tabs::HEIGHT;
const TOP_H: u32 = STRIP_H + ui::HEIGHT;

/// What makes a tab's engine for a window that opens none.
fn plain_maker() -> Box<dyn FnMut() -> Engine> {
    Box::new(|| {
        Engine::with_fonts(
            text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
                .expect("the bundled font parses"),
        )
    })
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
        Window::running(start, platform, false)
    }

    /// A window whose pages' scripts run, as `--enable-scripts` asks.
    fn with_scripts(start: Option<Url>) -> Window {
        Window::running(start, Platform::Other, true)
    }

    fn running(start: Option<Url>, platform: Platform, scripts: bool) -> Window {
        Window::kept(start, platform, scripts, None)
    }

    /// A window with scripts on whose engine keeps what it keeps in
    /// `memory` (M28), as `--profile` asks.
    fn on_profile(start: Option<Url>, memory: &profile::Memory) -> Window {
        let profile = profile::Profile::in_memory(memory).expect("a profile opens");
        Window::kept(start, Platform::Other, true, Some(profile))
    }

    fn kept(
        start: Option<Url>,
        platform: Platform,
        scripts: bool,
        profile: Option<profile::Profile>,
    ) -> Window {
        let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses");
        // Every tab of the window in one context (M37), as the shell's.
        let context = engine::Context::new();
        let (wake, woken) = channel();
        let maker = move || {
            let mut engine = Engine::with_fonts(fonts.clone());
            engine.join(&context);
            engine.set_scripting(scripts);
            let wake = wake.clone();
            engine.set_waker(move || {
                let _ = wake.send(());
            });
            engine
        };
        let mut engine = maker();
        if let Some(profile) = profile {
            engine.set_profile(profile).expect("the profile reads");
        }
        let board = Board::default();
        let mut session = Session::new(
            engine,
            Box::new(maker),
            Box::new(board.clone()),
            platform,
            start,
        );
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

/// The modifier of a text field's commands (select-all): the engine's
/// fields follow the system they were built for, Cmd on macOS, whatever
/// platform a test's session takes.
const FIELD_COMMAND: Modifiers = if cfg!(target_os = "macos") {
    Modifiers { logo: true, ..NONE }
} else {
    CTRL
};
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
/// the top of the page: at (50, 92) in the window, below the toolbar.
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
    window.session.pointer_moved(50.0, 92.0);
    let second = server.url("/second");
    assert_eq!(window.session.bubble_text(), Some(second.as_str()));
    assert_eq!(window.session.shape(), Shape::Hand);
    window.session.pointer_moved(50.0, 432.0);
    assert_eq!(window.session.bubble_text(), None);
    assert_eq!(window.session.shape(), Shape::Arrow);
    window.session.pointer_moved(300.0, 52.0);
    assert_eq!(window.session.shape(), Shape::Text, "over the field");
    window.click(50.0, 92.0);
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
    window.click(20.0, 52.0);
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
    window.click(50.0, 92.0);
    assert_eq!(
        window.session.bubble_text(),
        Some("mailto:someone@example.com")
    );
    window.session.pointer_moved(50.0, 432.0);
    let notice = window.session.bubble_text().expect("a notice").to_owned();
    assert!(notice.contains("mailto"), "{notice}");
    // A press on a link released off it follows nothing.
    window.press_at(50.0, 192.0, window.clock + Duration::from_secs(5));
    window.session.pointer_moved(50.0, 432.0);
    window.session.release(Button::Left);
    assert_eq!(*window.session.engine().load_state(), LoadState::Loaded);
    // A drag from the field over the page finds no link.
    window.chord("l", CTRL);
    window.press_at(300.0, 52.0, window.clock + Duration::from_secs(10));
    window.session.pointer_moved(50.0, 192.0);
    assert_eq!(window.session.bubble_text(), None);
    assert_eq!(window.session.shape(), Shape::Arrow);
    window.session.release(Button::Left);
    // The pointer leaving the window takes the bubble with it.
    window.session.pointer_moved(50.0, 192.0);
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
    window.click(300.0, 52.0);
    assert_eq!(window.session.focus(), Focus::Field);
    window.press_key(Key::End, NONE);
    let (end, ..) = window.session.ime_area().expect("the caret");
    let at = end - 3.0;
    let t = window.clock + Duration::from_secs(5);
    window.press_at(at, 52.0, t);
    window.session.release(Button::Left);
    assert_eq!(window.selected(), None, "a click places the caret");
    window.press_at(at, 52.0, t + Duration::from_millis(200));
    window.session.release(Button::Left);
    assert_eq!(window.selected().as_deref(), Some("path"));
    window.press_at(at, 52.0, t + Duration::from_millis(400));
    window.session.release(Button::Left);
    assert_eq!(
        window.selected().as_deref(),
        Some(server.url("/path").as_str())
    );
    // Too late, or too far, is a click again.
    window.press_at(at, 52.0, t + Duration::from_millis(1000));
    window.session.release(Button::Left);
    assert_eq!(window.selected(), None);
    window.press_at(at + 20.0, 52.0, t + Duration::from_millis(1100));
    window.session.release(Button::Left);
    assert_eq!(window.selected(), None);
    // Shift with a press selects from the caret.
    window.mods(SHIFT);
    window.press_at(at - 30.0, 52.0, t + Duration::from_millis(3000));
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
    window.click(400.0, 332.0);
    assert_eq!(window.session.focus(), Focus::Page);
    window.press_key(Key::Down, NONE);
    assert_eq!(window.session.engine().scroll_target(), LINE_PX);
    // A page is the page's height, below the toolbar, less a line.
    window.press_key(Key::Space, NONE);
    let page = (HEIGHT - TOP_H) as f32 - LINE_PX;
    assert_eq!(window.session.engine().scroll_target(), LINE_PX + page);
    window.press_key(Key::Home, NONE);
    assert_eq!(window.session.engine().scroll_target(), 0.0);
    // The wheel scrolls the page under the pointer, not over the
    // toolbar.
    window.session.pointer_moved(400.0, 52.0);
    window.session.wheel(LINE_PX);
    assert_eq!(window.session.engine().scroll_target(), 0.0);
    window.session.pointer_moved(400.0, 332.0);
    window.session.wheel(LINE_PX);
    assert_eq!(window.session.engine().scroll_target(), LINE_PX);
    window.press_key(Key::Home, NONE);
    window.frame();
    // The scrollbar is over the link at the page's right edge: no link
    // there. Its thumb, at the top, follows a drag.
    let bar_x = (WIDTH - 4) as f32;
    window.session.pointer_moved(bar_x, 92.0);
    assert_eq!(window.session.bubble_text(), None);
    window.session.pointer_moved(50.0, 92.0);
    assert!(window.session.bubble_text().is_some(), "the link");
    window.press_at(bar_x, TOP_H as f32 + 5.0, window.clock);
    window.session.pointer_moved(bar_x, TOP_H as f32 + 105.0);
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
    assert_eq!(at(&frame, 0, 0), 0xD6D6D6, "the tab strip");
    assert_eq!(
        at(&frame, 100, STRIP_H - 1),
        0xEDEDED,
        "the tab joined to it"
    );
    assert_eq!(at(&frame, 0, STRIP_H + 2), 0xEDEDED, "the toolbar");
    assert_eq!(at(&frame, 400, TOP_H - 1), 0xC9C9C9, "its edge");
    assert_eq!(at(&frame, 400, 300), 0x00FF00, "the page");
    assert_eq!(at(&frame, 2, HEIGHT - 30), 0x00FF00, "no bubble");
    assert_eq!(
        at(&frame, WIDTH - 20, HEIGHT - 2),
        0xFF0000,
        "the page's bottom"
    );
    assert!(!window.session.needs_frame());
    window.session.pointer_moved(50.0, 92.0);
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

#[test]
fn a_console_entry_prints_with_its_level() {
    use engine::{ConsoleEntry, ConsoleKind, ConsoleLevel, ConsoleSource};
    let line = |level| {
        console_line(&ConsoleEntry::of(
            ConsoleSource::Console,
            level,
            "x 1".to_owned(),
        ))
    };
    assert_eq!(line(ConsoleLevel::Log).as_deref(), Some("console: log x 1"));
    assert_eq!(
        line(ConsoleLevel::Info).as_deref(),
        Some("console: info x 1")
    );
    assert_eq!(
        line(ConsoleLevel::Warn).as_deref(),
        Some("console: warn x 1")
    );
    assert_eq!(
        line(ConsoleLevel::Error).as_deref(),
        Some("console: error x 1")
    );
    assert_eq!(
        line(ConsoleLevel::Debug).as_deref(),
        Some("console: debug x 1")
    );
    // A group's end and a clearing print nothing.
    let mut end = ConsoleEntry::of(ConsoleSource::Console, ConsoleLevel::Log, String::new());
    end.kind = ConsoleKind::GroupEnd;
    assert_eq!(console_line(&end), None);
}

#[test]
fn a_pages_scripts_title_the_window_and_a_realm_out_of_memory_says_so() {
    let server = serve(|path| match path {
        "/" => html(
            "<title>Before</title><script>console.log('hello'); document.title = 'x'</script>\
             <script>missing()</script><p>page",
        ),
        "/full" => html(
            "<script>var kept = []; for (let i = 0; ; i++) kept.push('x'.repeat(1 << 20) + i);</script>\
             <p>still here",
        ),
        _ => None,
    });
    // M24.7's acceptance, in the window: the title the script set, the
    // error noted.
    let mut window = Window::with_scripts(Some(server.url("/")));
    window.settle();
    assert!(
        window.session.title().starts_with("x "),
        "{}",
        window.session.title()
    );
    assert_eq!(
        window.session.engine().scripts(),
        engine::ScriptState::Running { errors: 1 }
    );
    // A realm that runs out of memory, at the default limits: the page
    // stays as drawn, and the bubble says why its scripts stopped.
    let mut window = Window::with_scripts(Some(server.url("/full")));
    window.settle();
    assert!(matches!(
        window.session.engine().scripts(),
        engine::ScriptState::Stopped { .. }
    ));
    assert_eq!(
        window.session.bubble_text(),
        Some("Scripts stopped on this page: out of memory")
    );
}

#[test]
fn what_a_pages_animation_callbacks_write_is_printed_with_the_frame() {
    // M30's particle demo logs from its animation frames, which no task
    // follows: the window prints what each frame's callbacks wrote.
    let server = serve(|path| match path {
        "/" => html(
            "<script>const again = () => { console.log('framed'); \
             requestAnimationFrame(again); }; requestAnimationFrame(again);</script>",
        ),
        _ => None,
    });
    let mut window = Window::with_scripts(Some(server.url("/")));
    window.settle();
    window.frame();
    assert!(window.session.engine_mut().take_console().is_empty());
}

#[test]
fn a_pages_timers_run_when_the_window_ticks() {
    let server = serve(|path| match path {
        "/" => html(
            "<title>Before</title><script>setTimeout(function () { \
             document.title = 'After'; console.log('ran'); });</script>",
        ),
        _ => None,
    });
    let mut window = Window::with_scripts(Some(server.url("/")));
    window.settle();
    assert!(window.session.title().starts_with("Before"));
    assert!(window.session.next_task_at().is_some());
    window.session.tick();
    assert!(window.session.title().starts_with("After"));
    assert_eq!(window.session.next_task_at(), None);
}

#[test]
fn the_page_hovers_and_sets_the_cursor_under_the_pointer() {
    let server = serve(|path| match path {
        "/" => html(
            "<style>body { margin: 0 } div { height: 40px } #m { cursor: move }              #m:hover { cursor: wait }</style><div id=m>m</div><div>x</div>",
        ),
        _ => None,
    });
    let mut window = Window::new(Some(server.url("/")), Platform::Other);
    window.settle();
    window.frame();
    let top = TOP_H as f32;
    // Hovered, the element takes its :hover style, whose cursor shows.
    window.session.pointer_moved(10.0, top + 10.0);
    assert_eq!(window.session.shape(), Shape::Page(engine::Cursor::Wait));
    window.session.pointer_moved(10.0, top + 50.0);
    assert_eq!(window.session.shape(), Shape::Arrow);
    window.session.pointer_left();
    let document = window.session.engine().document();
    assert!(
        document
            .subtree(document.root())
            .all(|node| !document.state(node).contains(engine::ElementState::HOVER))
    );
}

#[test]
fn a_covered_window_hides_its_page() {
    let server = serve(|path| match path {
        "/" => html(
            "<title>shown</title><script>document.addEventListener('visibilitychange', \
             function () { document.title = document.visibilityState; });</script>",
        ),
        _ => None,
    });
    let mut window = Window::with_scripts(Some(server.url("/")));
    window.settle();
    window.session.set_hidden(true);
    assert!(window.session.title().starts_with("hidden"));
    window.session.set_hidden(false);
    assert!(window.session.title().starts_with("visible"));
}

#[test]
fn a_page_loaded_before_the_first_frame_reads_the_windows_page_area() {
    // The window's size is known before a frame is drawn: a page that
    // arrives first reads the page's area under the toolbar (M25).
    let server = serve(|path| match path {
        "/" => html("<script>document.title = innerWidth + ' by ' + innerHeight</script>"),
        _ => None,
    });
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    let mut engine = Engine::with_fonts(fonts);
    engine.set_scripting(true);
    let (wake, woken) = channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    let mut session = Session::new(
        engine,
        plain_maker(),
        Box::new(Board::default()),
        Platform::Other,
        Some(server.url("/")),
    );
    session.resized(WIDTH, HEIGHT);
    let deadline = Instant::now() + WAIT;
    while *session.engine().load_state() == LoadState::Loading {
        let left = deadline
            .checked_duration_since(Instant::now())
            .expect("the page loads in time");
        woken.recv_timeout(left).expect("a fetch finishes in time");
        session.network();
    }
    assert_eq!(
        session.engine().title(),
        format!("{WIDTH} by {}", HEIGHT - TOP_H)
    );
}

#[test]
fn a_table_a_script_builds_is_styled_by_the_class_it_toggles() {
    // M25.5's acceptance, in the window: a page whose script builds a
    // table of five rows of five cells and toggles a class on it that
    // colours them; drawn as the class says, under the toolbar.
    let server = serve(|path| match path {
        "/" => html(
            "<style>body { margin: 0 } table { border-spacing: 0 } \
             td { width: 40px; height: 30px; padding: 0; background-color: #2f4f7f } \
             table.on td { background-color: #7f2f4f }</style><body>\
             <script>var table = document.createElement('table'); \
             for (var i = 0; i < 5; i++) { var row = table.appendChild(document.createElement('tr')); \
             for (var j = 0; j < 5; j++) { row.appendChild(document.createElement('td')); } } \
             document.body.appendChild(table); table.classList.toggle('on'); \
             table.classList.toggle('on'); table.classList.toggle('on');</script>",
        ),
        _ => None,
    });
    let mut window = Window::with_scripts(Some(server.url("/")));
    window.settle();
    let out = window.frame();
    let at = |x: u32, y: u32| out[((TOP_H + y) * WIDTH + x) as usize] & 0x00ff_ffff;
    for (x, y) in [(0, 0), (20, 15), (199, 149), (100, 75)] {
        assert_eq!(at(x, y), 0x7f2f4f, "({x}, {y})");
    }
    assert_eq!(at(200, 75), 0xffffff);
    assert_eq!(at(100, 150), 0xffffff);
}

#[test]
fn tab_moves_the_keyboard_between_the_field_and_the_page() {
    let server = serve(|path| match path {
        "/" => html(
            "<a id=a href=/x>a</a><a id=b href=/y>b</a><script>\
             addEventListener('blur', function (e) { \
               if (e.target === window) document.title += ' away'; });\
             addEventListener('keydown', function (e) { document.title = e.key; });</script>",
        ),
        _ => None,
    });
    let mut window = Window::with_scripts(Some(server.url("/")));
    window.settle();
    let focused = |window: &Window| {
        let engine = window.session.engine();
        engine
            .focused()
            .and_then(|node| engine.document().attr(node, "id"))
            .map(str::to_owned)
    };
    window.chord("l", CTRL);
    assert_eq!(window.session.focus(), Focus::Field);
    // From the field, Tab gives the page its first link, Shift+Tab its
    // last; past the last, the field has the keyboard back.
    window.press_key(Key::Tab, NONE);
    assert_eq!(window.session.focus(), Focus::Page);
    assert_eq!(focused(&window).as_deref(), Some("a"));
    window.press_key(Key::Tab, NONE);
    assert_eq!(focused(&window).as_deref(), Some("b"));
    window.press_key(Key::Tab, NONE);
    assert_eq!(window.session.focus(), Focus::Field);
    assert_eq!(focused(&window), None);
    assert!(window.session.title().starts_with("Tab away"));
    window.press_key(Key::Tab, SHIFT);
    assert_eq!(focused(&window).as_deref(), Some("b"));
    // The page hears its keys, not the browser's.
    window.press_key(Key::Down, NONE);
    assert!(window.session.title().starts_with("ArrowDown"));
    window.press_key(Key::F5, NONE);
    assert!(window.session.title().starts_with("ArrowDown"));
}

#[test]
fn a_pages_field_takes_typing_the_clipboard_and_an_input_methods_text() {
    let server = serve(|path| match path {
        "/" => html("<input id=t>"),
        _ => None,
    });
    let mut window = Window::with_scripts(Some(server.url("/")));
    window.settle();
    let value = |window: &Window| {
        let engine = window.session.engine();
        let document = engine.document();
        let field = document
            .subtree(document.root())
            .find(|&node| document.attr(node, "id") == Some("t"))
            .expect("the field");
        // What the user typed is the field's own value.
        document
            .control(field)
            .and_then(|state| state.value.clone())
            .unwrap_or_default()
    };
    assert_eq!(window.session.focus(), Focus::Page);
    window.press_key(Key::Tab, NONE);
    window.type_text("hello");
    assert_eq!(value(&window), "hello");
    // Select-all on the page is the field's; copied, then pasted at the
    // end.
    window.chord("a", FIELD_COMMAND);
    window.chord("c", CTRL);
    assert_eq!(window.board.text(), "hello");
    window.press_key(Key::End, NONE);
    window.chord("v", CTRL);
    assert_eq!(value(&window), "hellohello");
    window.session.commit("!");
    assert_eq!(value(&window), "hellohello!");
    // The input method's window stands by the caret, below the toolbar.
    let (_, y, _, h) = window
        .session
        .ime_area()
        .expect("an input method is wanted");
    assert!(y >= TOP && h > 0.0);
    // Cut takes the selection to the clipboard.
    window.chord("a", FIELD_COMMAND);
    window.chord("x", CTRL);
    assert_eq!(window.board.text(), "hellohello!");
    assert_eq!(value(&window), "");
}

/// M26's to-do list (the design note's acceptance): a field, a button
/// and a list, a script adding an entry from the field on Enter or on
/// the button's click, marking an entry done by a click on it, and
/// taking it out by its own button. Laid out at known places, below
/// the toolbar.
const TODO: &str = "<style>body { margin: 0 } \
     #new { position: absolute; left: 0; top: 0; width: 200px; height: 24px; margin: 0 } \
     #add { position: absolute; left: 210px; top: 0; width: 60px; height: 24px; margin: 0 } \
     ul { position: absolute; left: 0; top: 40px; width: 300px; margin: 0; padding: 0; list-style: none } \
     li { height: 20px; margin-bottom: 4px; background-color: #2f4f7f } \
     li.done { background-color: #7f2f4f } \
     li button { float: right; width: 20px; height: 20px; margin: 0; padding: 0; border: 0; \
       background-color: #202020 }</style>\
     <input id=new><button id=add>Add</button><ul id=list></ul><script>\
     var list = document.getElementById('list'), field = document.getElementById('new');\
     function add() { if (!field.value) return; var li = document.createElement('li');\
       li.dataset.text = field.value; var x = document.createElement('button');\
       x.addEventListener('click', function (e) { e.stopPropagation(); li.remove(); });\
       li.appendChild(x); li.addEventListener('click', function () { li.classList.toggle('done'); });\
       list.appendChild(li); field.value = ''; document.title = list.children.length + ' to do'; }\
     document.getElementById('add').addEventListener('click', add);\
     field.addEventListener('keydown', function (e) { if (e.key === 'Enter') add(); });</script>";

#[test]
fn the_to_do_list_takes_entries_marks_them_done_and_lets_them_go() {
    let server = serve(|path| match path {
        "/" => html(TODO),
        _ => None,
    });
    let mut window = Window::with_scripts(Some(server.url("/")));
    window.settle();
    let top = TOP_H as f32;
    let entries = |window: &Window| {
        let engine = window.session.engine();
        let document = engine.document();
        document
            .subtree(document.root())
            .filter(|&node| document.tag(node) == Some("li"))
            .map(|node| {
                let done = document
                    .attr(node, "class")
                    .is_some_and(|class| class.contains("done"));
                format!(
                    "{}{}",
                    document.attr(node, "data-text").unwrap_or(""),
                    if done { " done" } else { "" }
                )
            })
            .collect::<Vec<_>>()
    };
    // Typed and entered with the keyboard.
    window.click(100.0, top + 12.0);
    window.type_text("milk");
    window.press_key(Key::Enter, NONE);
    // Typed and added with the button.
    window.type_text("eggs");
    window.click(240.0, top + 12.0);
    assert_eq!(entries(&window), ["milk", "eggs"]);
    assert!(window.session.title().starts_with("2 to do"));
    // The first marked done by a click, drawn in its colour.
    window.click(100.0, top + 50.0);
    assert_eq!(entries(&window), ["milk done", "eggs"]);
    let out = window.frame();
    let at = |x: u32, y: u32| out[((TOP_H + y) * WIDTH + x) as usize] & 0x00ff_ffff;
    assert_eq!(at(100, 50), 0x7f2f4f, "done");
    assert_eq!(at(100, 74), 0x2f4f7f, "not done");
    // The second taken out by its button: the click reaches the
    // button, not the entry.
    window.click(290.0, top + 74.0);
    assert_eq!(entries(&window), ["milk done"]);
    let out = window.frame();
    let at = |x: u32, y: u32| out[((TOP_H + y) * WIDTH + x) as usize] & 0x00ff_ffff;
    assert_eq!(at(100, 74), 0xffffff, "gone");
}

/// A search form and its results page, from the server: the query the
/// results page was asked for in its title.
fn search_server() -> TestServer {
    serve(|path| {
        if path == "/" {
            return html(
                "<style>body { margin: 0 } input { position: absolute; left: 0; top: 0; \
                 width: 200px; height: 24px; margin: 0 } button { position: absolute; \
                 left: 210px; top: 0; width: 60px; height: 24px; margin: 0 }</style>\
                 <form action=/search><input name=q><input type=hidden name=from value=page>\
                 <button>Search</button></form><script>\
                 document.forms[0].addEventListener('submit', function () { \
                   document.forms[0].elements.from.value = 'script'; });</script>",
            );
        }
        let query = path.strip_prefix("/search?")?;
        html(&format!("<title>results for {query}</title>"))
    })
}

#[test]
fn the_search_form_is_sent_with_enter_and_with_its_button_scripts_on_and_off() {
    for scripts in [false, true] {
        let server = search_server();
        let mut window = Window::running(Some(server.url("/")), Platform::Other, scripts);
        window.settle();
        let top = TOP_H as f32;
        let from = if scripts { "script" } else { "page" };
        // Typed, then Enter: implicit submission.
        window.click(100.0, top + 12.0);
        window.type_text("silver browser");
        window.press_key(Key::Enter, NONE);
        window.settle();
        assert_eq!(
            window.session.engine().title(),
            format!("results for q=silver+browser&from={from}"),
            "scripts {scripts}"
        );
        // Back, then the button.
        window.press_key(Key::Left, ALT);
        window.settle();
        window.click(100.0, top + 12.0);
        window.type_text("again");
        window.click(240.0, top + 12.0);
        window.settle();
        assert!(
            window
                .session
                .engine()
                .title()
                .starts_with("results for q="),
            "scripts {scripts}: {}",
            window.session.engine().title()
        );
    }
}

/// M27's acceptance page (docs/design/fetch.md, "Acceptance"): a button
/// that signs in by a `fetch` POST, the server answering with a Lax,
/// HttpOnly session cookie; a second request, which the cookie
/// authorizes, naming the user; then data from a second origin by CORS
/// with credentials, twice, its own cookie set by the first answer and
/// sent with the second. The title says what came, and a box as wide as
/// fifty pixels a visit shows it.
const SIGN_IN: &str = "<style>body { margin: 0 } button { position: absolute; left: 0; top: 0; \
     width: 120px; height: 30px; margin: 0 } #box { position: absolute; left: 0; top: 40px; \
     height: 20px; background-color: #2f6f4f }</style>\
     <button id=go>Sign in</button><div id=box></div><script>\
     document.getElementById('go').addEventListener('click', async function () {\
       const body = new URLSearchParams({user: 'ada', password: 'lovelace'});\
       const signed = await fetch('/login', {method: 'POST', body: body});\
       if (!signed.ok) { document.title = 'refused'; return; }\
       const me = await (await fetch('/me')).json();\
       const visits = [];\
       for (let i = 0; i < 2; i++) {\
         const r = await fetch(OTHER + '/data', {credentials: 'include'});\
         visits.push((await r.json()).visits);\
       }\
       document.getElementById('box').style.width = (visits[1] * 50) + 'px';\
       document.title = 'Signed in as ' + me.name + ', visits ' + visits.join(' then ') +\
         ', cookie ' + JSON.stringify(document.cookie);\
     });</script>";

/// M28's acceptance page: it counts its visits in `localStorage` and
/// in a cookie with an expiry, and says how many items the tab's
/// `sessionStorage` had, in its title.
const VISITS: &str = "<!DOCTYPE html><script>\
    const n = Number(localStorage.getItem('visits') || 0) + 1;\
    localStorage.setItem('visits', n);\
    const c = Number((document.cookie.match(/visits=([0-9]+)/) || [0, 0])[1]) + 1;\
    document.cookie = 'visits=' + c + '; Max-Age=3600';\
    const s = sessionStorage.length;\
    sessionStorage.setItem('here', '1');\
    document.title = 'visits ' + n + ' cookie ' + c + ' session ' + s;\
    </script>";

#[test]
fn a_page_counts_its_visits_across_windows_on_one_profile() {
    let server = TestServer::plain(|_| Reply::ok("text/html", VISITS.as_bytes()));
    let memory = profile::Memory::new();
    for visit in 1..=2 {
        let mut window = Window::on_profile(Some(server.url("/")), &memory);
        window.settle();
        assert_eq!(
            window.session.engine().title(),
            format!("visits {visit} cookie {visit} session 0")
        );
        // The window closes: the shell writes the profile.
        window.session.flush_profile().unwrap();
    }
    // A window with no profile starts again.
    let mut window = Window::with_scripts(Some(server.url("/")));
    window.settle();
    assert_eq!(
        window.session.engine().title(),
        "visits 1 cookie 1 session 0"
    );
}

#[test]
fn a_page_signs_in_by_fetch_and_reads_another_origin_by_cors_with_credentials() {
    // The other origin, of another site: it allows the page's origin,
    // with credentials, and counts a visitor by its own cookie.
    let other = TestServer::plain(|request| {
        let origin = request.header("Origin").unwrap_or("").to_owned();
        let seen = request
            .header("Cookie")
            .is_some_and(|c| c.contains("seen=1"));
        let mut fields = vec![
            ("Content-Type", "application/json".to_owned()),
            ("Access-Control-Allow-Origin", origin),
            ("Access-Control-Allow-Credentials", "true".to_owned()),
        ];
        if !seen {
            fields.push((
                "Set-Cookie",
                "seen=1; SameSite=None; Secure; Path=/".to_owned(),
            ));
        }
        let fields: Vec<(&str, &str)> = fields.iter().map(|(n, v)| (*n, v.as_str())).collect();
        let body = if seen {
            "{\"visits\":2}"
        } else {
            "{\"visits\":1}"
        };
        Reply::with_fields(200, &fields, body.as_bytes())
    });
    let page = SIGN_IN.replace("OTHER", &format!("'http://localhost:{}'", other.port()));
    let server = TestServer::plain(move |request| {
        let cookie = request.header("Cookie").unwrap_or("");
        match (request.method.as_str(), request.target.as_str()) {
            ("GET", "/") => Reply::ok("text/html", page.as_bytes()),
            ("POST", "/login") if request.body == b"user=ada&password=lovelace" => {
                Reply::with_fields(
                    200,
                    &[(
                        "Set-Cookie",
                        "session=s3cret; HttpOnly; SameSite=Lax; Path=/",
                    )],
                    b"",
                )
            }
            ("GET", "/me") if cookie.contains("session=s3cret") => {
                Reply::ok("application/json", b"{\"name\":\"ada\"}")
            }
            _ => Reply::with_fields(403, &[], b""),
        }
    });
    let mut window = Window::with_scripts(Some(server.url("/")));
    window.settle();
    let top = TOP_H as f32;
    window.click(60.0, top + 15.0);
    // The answers come as the network brings them, each a task the
    // window's tick runs.
    let deadline = Instant::now() + WAIT;
    while !window.session.title().starts_with("Signed in") {
        assert!(
            Instant::now() < deadline,
            "not signed in: {}",
            window.session.title()
        );
        let _ = window.woken.recv_timeout(Duration::from_millis(20));
        window.session.network();
        window.session.tick();
        window.frame();
    }
    assert!(
        window
            .session
            .title()
            .starts_with("Signed in as ada, visits 1 then 2, cookie \"\""),
        "{}",
        window.session.title()
    );
    let out = window.frame();
    let at = |x: u32, y: u32| out[((TOP_H + y) * WIDTH + x) as usize] & 0x00ff_ffff;
    assert_eq!(at(90, 50), 0x2f6f4f, "two visits wide");
    assert_eq!(at(110, 50), 0xffffff, "and no wider");
    // The session cookie went with the second request only; the other
    // origin's own went with its second answer's request, both asked
    // from the page's origin.
    let asked = server.requests();
    let me = asked.iter().find(|r| r.target == "/me").expect("/me asked");
    assert!(
        me.header("Cookie")
            .is_some_and(|c| c.contains("session=s3cret"))
    );
    let login = asked
        .iter()
        .find(|r| r.target == "/login")
        .expect("/login asked");
    assert_eq!(
        login.header("Content-Type"),
        Some("application/x-www-form-urlencoded;charset=UTF-8")
    );
    let data: Vec<_> = other.requests();
    assert_eq!(data.len(), 2);
    assert_eq!(data[0].header("Cookie"), None);
    assert_eq!(data[1].header("Cookie"), Some("seen=1"));
    let origin = format!("http://127.0.0.1:{}", server.port());
    assert!(
        data.iter()
            .all(|r| r.header("Origin") == Some(origin.as_str()))
    );
}

#[test]
fn the_window_is_drawn_at_its_scale_and_its_pointer_taken_by_it() {
    // M32.4: the toolbar and the page at twice the pixels, the
    // positions the session takes in CSS pixels.
    let mut window = Window::new(None, Platform::Other);
    let once = window.frame();
    window.session.set_scale(2.0, WIDTH, HEIGHT);
    let twice = window.frame();
    let w = WIDTH as usize;
    let edge = TOP_H as usize - 1;
    // The toolbar's bottom edge, a row at 1, two rows twice as far down
    // at 2.
    assert_eq!(twice[(2 * edge) * w], once[edge * w]);
    assert_eq!(twice[(2 * edge + 1) * w], once[edge * w]);
    assert_ne!(twice[edge * w], once[edge * w]);
    // The field ends short of the toolbar's right edge, in CSS pixels:
    // the bar's own colour there, as above the buttons.
    let bar = STRIP_H as usize;
    assert_eq!(twice[(2 * bar + 40) * w + w - 4], once[(bar + 2) * w + 10]);
    // The page's viewport is the window's CSS pixels under the toolbar.
    let (vw, vh) = window.session.engine().viewport();
    assert_eq!((vw, vh), (WIDTH / 2, HEIGHT / 2 - TOP_H));
    // A pointer at device (700, 104) is at CSS (350, 52): over the
    // address field, where at a pixel to a pixel it would be on the page.
    window.session.pointer_moved(700.0, 104.0);
    assert_eq!(window.session.shape(), Shape::Text);
}

#[test]
fn a_sites_prompt_shows_under_the_toolbar_and_its_buttons_answer_it() {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    let printer = serve(|_| {
        Some(Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/plain"),
                ("Access-Control-Allow-Origin", "*"),
            ],
            b"local",
        ))
    });
    // An app on this machine, its space its own.
    let app = serve(|_| html("app"));
    let (port, app_port) = (printer.port(), app.port());
    let page = serve(move |path| match path {
        "/" => html(&format!(
            "<body style='margin:0;background-color:#00ff00'><script>\
             const ask = (at) => fetch('http://' + at + '/s').catch(() => {{}});\
             ask('printer.test:{port}').then(() => ask('127.0.0.1:{app_port}'));</script>"
        )),
        _ => None,
    });
    // The page's server public, the printer's local (M33.4).
    let here = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let config = net::transport::Config::default()
        .resolve("printer.test", here)
        .treat_as(
            SocketAddr::new(here, page.port()),
            net::AddressSpace::Public,
        )
        .treat_as(SocketAddr::new(here, port), net::AddressSpace::Local);
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    let mut engine = Engine::with_fonts(fonts);
    engine.set_scripting(true);
    engine.set_network(net::Client::new(config));
    let (wake, woken) = channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    let board = Board::default();
    let start = Some(page.url("/"));
    let mut session = Session::new(
        engine,
        plain_maker(),
        Box::new(board.clone()),
        Platform::Other,
        start,
    );
    session.resized(WIDTH, HEIGHT);
    let mut window = Window {
        session,
        woken,
        board,
        clock: Duration::from_secs(1),
    };
    window.wait_until(|session| session.prompt().is_some());
    let prompt = window.session.prompt().expect("a prompt").clone();
    assert_eq!(prompt.permission, engine::Permission::LocalNetwork);
    let frame = window.frame();
    let at = |frame: &[u32], x: u32, y: u32| frame[(y * WIDTH + x) as usize];
    let bar = TOP_H + ui::prompt::HEIGHT / 2;
    assert_eq!(at(&frame, 3, bar), 0xFFF8E1, "the prompt's bar");
    assert_eq!(
        at(&frame, 3, TOP_H + ui::prompt::HEIGHT + 5),
        0x00FF00,
        "the page under it"
    );
    assert!(!window.session.needs_frame());
    assert!(printer.requests().is_empty(), "nothing sent yet");
    // Allow: the request goes, and the bar with the prompt.
    window.click((WIDTH - 136) as f32, bar as f32);
    assert!(window.session.prompt().is_none());
    assert!(window.session.needs_frame(), "the bar goes");
    window.wait_until(|_| !printer.requests().is_empty());
    let frame = window.frame();
    assert_eq!(at(&frame, 3, bar), 0x00FF00, "no bar");
    // Escape dismisses the next, which the page's tasks ask for, the
    // shell woken when it opens: nothing sent.
    let deadline = Instant::now() + WAIT;
    while window.session.prompt().is_none() {
        if window.session.next_task_at().is_some() {
            window.session.tick();
            continue;
        }
        let left = deadline
            .checked_duration_since(Instant::now())
            .expect("the second prompt in time");
        window.woken.recv_timeout(left).expect("woken in time");
        window.session.network();
    }
    assert_eq!(
        window.session.prompt().map(|prompt| prompt.permission),
        Some(engine::Permission::LoopbackNetwork)
    );
    window.press_key(Key::Escape, NONE);
    assert!(window.session.prompt().is_none());
    assert_eq!(printer.requests().len(), 1);
    assert!(app.requests().is_empty());
}

impl Window {
    /// Runs the window as the shell does, its tasks and the DevTools
    /// panel's too, until `done` holds.
    fn run_until(&mut self, done: impl Fn(&mut Session) -> bool) {
        let deadline = Instant::now() + WAIT;
        while !done(&mut self.session) {
            if Instant::now() >= deadline {
                let text = panel_text(&mut self.session, "panes");
                panic!("the session gets there in time: {text:?}");
            }
            self.session.tick();
            self.session.network();
            self.frame();
            let _ = self.woken.recv_timeout(Duration::from_millis(5));
        }
    }
}

/// The text of the element of id `id` in the DevTools panel's page.
fn panel_text(session: &mut Session, id: &str) -> Option<String> {
    let document = session.panel_mut()?.document();
    document
        .subtree(document.root())
        .find(|&node| document.attr(node, "id") == Some(id))
        .map(|node| document.text_content(node))
}

#[test]
fn f12_opens_the_devtools_panel_under_the_page_and_closes_it() {
    let server = two_pages();
    let mut window = Window::new(Some(server.url("/")), Platform::Other);
    window.settle();
    let whole = window.session.engine().viewport();
    assert_eq!(whole, (WIDTH, HEIGHT - TOP_H));
    window.press_key(Key::F12, NONE);
    // The panel takes the window's lower part and the keyboard; the page
    // is laid out to what is left.
    assert_eq!(window.session.focus(), Focus::Panel);
    let panel_height = ((HEIGHT - TOP_H) as f32 * PANEL_SHARE).round() as u32;
    assert_eq!(
        window.session.engine().viewport(),
        (WIDTH, HEIGHT - TOP_H - panel_height)
    );
    assert_eq!(
        window.session.panel_mut().map(|panel| panel.viewport()),
        Some((WIDTH, panel_height))
    );
    // Through the router, the panel names the page it inspects.
    let named = format!("First - {}", server.url("/"));
    window.run_until(|session| panel_text(session, "inspected").as_deref() == Some(&named));
    // The panel is drawn under the page: its rows are not the page's.
    let frame = window.frame();
    let top = (HEIGHT - panel_height) as usize;
    let row = |y: usize| frame[y * WIDTH as usize..(y + 1) * WIDTH as usize].to_vec();
    // Its first row is the panel page's top border, its line colour,
    // whole: the panel's page does not scroll.
    let border = row(top);
    let differ: Vec<_> = border
        .iter()
        .enumerate()
        .filter(|&(_, &p)| p != 0xccd0da)
        .collect();
    assert!(differ.is_empty(), "{differ:x?}");
    // A navigation is told to the panel, which names the new page.
    window.click(50.0, 92.0);
    let named = format!("Second - {}", server.url("/second"));
    window.run_until(|session| panel_text(session, "inspected").as_deref() == Some(&named));
    // A press on the page gives it the keyboard; one on the panel, the
    // panel; F12 closes the panel from either.
    window.click(400.0, (HEIGHT - panel_height - 20) as f32);
    assert_eq!(window.session.focus(), Focus::Page);
    window.click(400.0, (HEIGHT - 20) as f32);
    assert_eq!(window.session.focus(), Focus::Panel);
    window.press_key(Key::F12, NONE);
    assert!(window.session.panel_mut().is_none());
    assert_eq!(window.session.focus(), Focus::Page);
    assert_eq!(window.session.engine().viewport(), whole);
    // Ctrl+Shift+I opens it again.
    window.chord(
        "I",
        Modifiers {
            ctrl: true,
            shift: true,
            ..NONE
        },
    );
    assert!(window.session.panel_mut().is_some());
}

#[test]
fn the_router_answers_the_browsers_requests_and_passes_the_pages_answers_alone() {
    use devtools::Json;
    let server = two_pages();
    let mut router = crate::devtools::Router::default();
    let mut panel = Recorder::default();
    let mut page = Recorder {
        url: Some(server.url("/")),
        ..Recorder::default()
    };
    // The first route tells the panel which page it inspects.
    router.route(&mut panel, &mut page);
    let told = panel.sent.remove(0);
    assert!(told.starts_with(r#"{"event":"page.navigated""#), "{told}");
    panel.out = vec![
        r#"{"id":1,"method":"browser.version"}"#.to_owned(),
        r#"{"id":2,"method":"page.info"}"#.to_owned(),
        r#"{"id":3,"method":"browser.nothing"}"#.to_owned(),
        "not a request".to_owned(),
    ];
    page.out = vec![
        // An answer to a request the page was not given, and one to a
        // request it was.
        r#"{"id":1,"result":"forged"}"#.to_owned(),
        r#"{"id":2,"result":{"title":"First"}}"#.to_owned(),
        r#"{"id":2,"result":"again"}"#.to_owned(),
        r#"{"event":"dom.changed","params":{}}"#.to_owned(),
        r#"{"neither":true}"#.to_owned(),
    ];
    router.route(&mut panel, &mut page);
    assert_eq!(page.asked, [r#"{"id":2,"method":"page.info"}"#]);
    let version = Json::parse(&panel.sent[0]).expect("JSON");
    assert_eq!(
        version
            .get("result")
            .and_then(|r| r.get("name"))
            .and_then(Json::as_str),
        Some("Silver Browser")
    );
    assert_eq!(
        panel.sent[1],
        r#"{"id":3,"error":"no method browser.nothing"}"#
    );
    assert_eq!(
        &panel.sent[2..],
        [
            r#"{"id":2,"result":{"title":"First"}}"#,
            r#"{"event":"dom.changed","params":{}}"#,
        ]
    );
    // A request still waiting when the page navigates is answered so.
    panel.sent.clear();
    panel.out = vec![r#"{"id":9,"method":"dom.tree"}"#.to_owned()];
    router.route(&mut panel, &mut page);
    page.url = Some(server.url("/second"));
    router.route(&mut panel, &mut page);
    assert_eq!(panel.sent[0], r#"{"id":9,"error":"the page navigated"}"#);
    assert!(panel.sent[1].contains("/second"), "{:?}", panel.sent);
}

/// A view of the tests' own for the router: what it was sent and asked,
/// and what it has to give.
#[derive(Default)]
struct Recorder {
    url: Option<Url>,
    log: Option<std::sync::Arc<engine::network_log::NetworkLog>>,
    storage: Vec<String>,
    out: Vec<String>,
    sent: Vec<String>,
    asked: Vec<String>,
}

impl crate::devtools::End for Recorder {
    fn url(&self) -> Option<&Url> {
        self.url.as_ref()
    }
    fn devtools_send(&mut self, message: String) {
        self.sent.push(message);
    }
    fn take_devtools(&mut self) -> Vec<String> {
        std::mem::take(&mut self.out)
    }
    fn inspect(&mut self, message: String) {
        self.asked.push(message);
    }
    fn take_inspected(&mut self) -> Vec<String> {
        std::mem::take(&mut self.out)
    }
    fn network_log(&self) -> Option<std::sync::Arc<engine::network_log::NetworkLog>> {
        self.log.clone()
    }
    fn storage_inspect(&mut self, message: &str) -> String {
        self.storage.push(message.to_owned());
        r#"{"id":1,"result":[]}"#.to_owned()
    }
}

#[test]
fn the_router_keeps_the_console_and_gives_it_to_the_panel() {
    use devtools::Json;
    use engine::{ConsoleEntry, ConsoleLevel, ConsoleSource};
    let mut router = crate::devtools::Router::default();
    let first = Url::parse("https://a.test/").expect("a URL");
    let none: Option<&mut Recorder> = None;
    router.watch(Some(&first), none);
    let mut logged = ConsoleEntry::of(ConsoleSource::Console, ConsoleLevel::Warn, "w 1".to_owned());
    logged.previews = r#"[{"type":"string","value":"w"},{"type":"number","value":1}]"#.to_owned();
    logged.url = "https://a.test/".to_owned();
    logged.line = 3;
    let mut forged = ConsoleEntry::of(ConsoleSource::Console, ConsoleLevel::Log, "f".to_owned());
    forged.previews = "not JSON".to_owned();
    // Written while the panel is closed, the entries are kept.
    router.log(vec![logged, forged], None::<&mut Recorder>);
    let mut panel = Recorder::default();
    let mut page = Recorder {
        url: Some(first.clone()),
        ..Recorder::default()
    };
    panel.out = vec![r#"{"id":1,"method":"console.history"}"#.to_owned()];
    router.route(&mut panel, &mut page);
    assert!(page.asked.is_empty(), "the browser answers it");
    let history = Json::parse(&panel.sent[0]).expect("JSON");
    let entries = history
        .get("result")
        .and_then(Json::as_array)
        .expect("entries");
    assert_eq!(entries.len(), 2);
    let get = |entry: &Json, name: &str| entry.get(name).cloned();
    assert_eq!(get(&entries[0], "level"), Some(Json::from("warn")));
    assert_eq!(get(&entries[0], "line"), Some(Json::from(3u32)));
    assert_eq!(
        entries[0]
            .get("args")
            .and_then(Json::as_array)
            .map(<[Json]>::len),
        Some(2)
    );
    // Previews that are not JSON are left out, the text kept.
    assert_eq!(get(&entries[1], "args"), Some(Json::Array(Vec::new())));
    assert_eq!(get(&entries[1], "text"), Some(Json::from("f")));
    // While the panel is open, each entry is told as it comes.
    panel.sent.clear();
    let entry = ConsoleEntry::of(ConsoleSource::Network, ConsoleLevel::Error, "n".to_owned());
    router.log(vec![entry], Some(&mut panel));
    assert!(panel.sent[0].starts_with(r#"{"event":"console.entry","params":{"level":"error""#));
    // The panel's clearing clears the history; a navigation does too.
    panel.sent.clear();
    panel.out = vec![
        r#"{"id":2,"method":"console.clear"}"#.to_owned(),
        r#"{"id":3,"method":"console.history"}"#.to_owned(),
    ];
    router.route(&mut panel, &mut page);
    assert_eq!(panel.sent[1], r#"{"id":3,"result":[]}"#);
    router.log(
        vec![ConsoleEntry::of(
            ConsoleSource::Console,
            ConsoleLevel::Log,
            "x".to_owned(),
        )],
        None::<&mut Recorder>,
    );
    page.url = Some(Url::parse("https://b.test/").expect("a URL"));
    panel.sent.clear();
    panel.out = vec![r#"{"id":4,"method":"console.history"}"#.to_owned()];
    router.route(&mut panel, &mut page);
    assert_eq!(
        panel.sent.last().map(String::as_str),
        Some(r#"{"id":4,"result":[]}"#)
    );
}

/// The text of the DevTools panel's console lines.
fn panel_console(session: &mut Session) -> String {
    panel_text(session, "console-lines").unwrap_or_default()
}

#[test]
fn the_panels_console_shows_the_pages_entries_and_evaluates() {
    let server = serve(|path| match path {
        "/" => html(
            "<title>Logged</title><script>console.log('hello', { a: 1 }); \
             console.error(new TypeError('broken'))</script>",
        ),
        _ => None,
    });
    let mut window = Window::with_scripts(Some(server.url("/")));
    window.settle();
    window.press_key(Key::F12, NONE);
    window.run_until(|session| {
        let text = panel_console(session);
        text.contains("hello {a: 1}") && text.contains("TypeError: broken")
    });
    // What the page writes now comes as it is written.
    let later =
        r#"{"id":99,"method":"console.evaluate","params":{"expression":"console.info('later')"}}"#;
    View::inspect(window.session.engine_mut(), later.to_owned());
    window.run_until(|session| panel_console(session).contains("later"));
    // An expression typed at the prompt, at the console's foot, runs in
    // the page: the Console tab first, the second of the panel's tabs.
    let panel_top = HEIGHT - ((HEIGHT - TOP_H) as f32 * PANEL_SHARE).round() as u32;
    window.click(110.0, (panel_top + 10) as f32);
    window.run_until(|session| {
        panel_text(session, "pane-console").is_some()
            && session.panel_mut().is_some_and(|panel| {
                let document = panel.document();
                document
                    .subtree(document.root())
                    .find(|&node| document.attr(node, "id") == Some("pane-console"))
                    .and_then(|node| document.attr(node, "class"))
                    .is_some_and(|class| !class.contains("off"))
            })
    });
    window.click(400.0, (HEIGHT - 6) as f32);
    assert_eq!(window.session.focus(), Focus::Panel);
    window.type_text("6*7");
    window.press_key(Key::Enter, NONE);
    window.run_until(|session| {
        let text = panel_console(session);
        text.contains("> 6*7") && text.contains("42")
    });
}

#[test]
fn the_panels_inspector_shows_the_tree_and_a_picked_nodes_rules() {
    let server = serve(|path| match path {
        "/" => html(
            "<title>Inspected</title><style>.x { color: green }</style>\
             <body style='margin: 0'><p class=x style='height: 100px'>a paragraph</p>",
        ),
        _ => None,
    });
    let mut window = Window::with_scripts(Some(server.url("/")));
    window.settle();
    window.press_key(Key::F12, NONE);
    window.run_until(|session| {
        panel_text(session, "elements-tree").is_some_and(|tree| tree.contains("class=\"x\""))
    });
    // A pick, as the panel's button asks for it, then a press on the
    // paragraph: the panel selects it and shows its rules.
    let pick = r#"{"id":77,"method":"dom.pick","params":{"on":true}}"#;
    View::inspect(window.session.engine_mut(), pick.to_owned());
    window.click(100.0, 92.0);
    window.run_until(|session| {
        panel_text(session, "elements-styles")
            .is_some_and(|styles| styles.contains(".x {") && styles.contains("color: green;"))
    });
    window.run_until(|session| {
        panel_text(session, "elements-computed")
            .is_some_and(|computed| computed.contains("rgb(0, 128, 0)"))
    });
}

#[test]
fn the_router_gives_the_panel_the_tabs_network_log() {
    use devtools::Json;
    use net::observe::{Observed, Observer};
    let log = std::sync::Arc::new(engine::network_log::NetworkLog::default());
    let url = Url::parse("https://a.test/s.css").expect("a URL");
    let mut router = crate::devtools::Router::default();
    let mut panel = Recorder::default();
    let mut page = Recorder {
        url: Some(url.clone()),
        log: Some(log.clone()),
        ..Recorder::default()
    };
    router.route(&mut panel, &mut page);
    panel.sent.clear();
    log.observe(
        3,
        Observed::Request {
            method: "GET",
            url: &url,
            headers: &net::HeaderList::new(),
            destination: net::Destination::Style,
            initiator: None,
        },
    );
    // A change of the log is told once.
    router.route(&mut panel, &mut page);
    router.route(&mut panel, &mut page);
    assert_eq!(panel.sent, [r#"{"event":"network.updated","params":{}}"#]);
    panel.sent.clear();
    panel.out = vec![
        r#"{"id":1,"method":"network.list"}"#.to_owned(),
        r#"{"id":2,"method":"network.entry","params":{"id":3}}"#.to_owned(),
        r#"{"id":3,"method":"network.entry","params":{"id":9}}"#.to_owned(),
        r#"{"id":4,"method":"network.clear"}"#.to_owned(),
    ];
    router.route(&mut panel, &mut page);
    assert!(page.asked.is_empty(), "the browser answers them");
    let list = Json::parse(&panel.sent[0]).expect("JSON");
    let entries = list.get("result").and_then(Json::as_array).expect("a list");
    assert_eq!(entries[0].get("type").and_then(Json::as_str), Some("style"));
    let entry = Json::parse(&panel.sent[1]).expect("JSON");
    assert_eq!(
        entry
            .get("result")
            .and_then(|e| e.get("url"))
            .and_then(Json::as_str),
        Some(url.as_str())
    );
    assert_eq!(panel.sent[2], r#"{"id":3,"error":"no such request"}"#);
    assert_eq!(log.list(), Json::Array(Vec::new()));
}

#[test]
fn the_panels_network_log_lists_the_pages_requests() {
    let server = serve(|path| match path {
        "/" => html("<title>Requests</title><link rel=stylesheet href=/look.css>"),
        "/look.css" => Some(Reply::ok("text/css", b"p { color: red }")),
        _ => None,
    });
    let mut window = Window::new(Some(server.url("/")), Platform::Other);
    window.settle();
    window.press_key(Key::F12, NONE);
    window.run_until(|session| {
        panel_text(session, "network-list").is_some_and(|list| list.contains("look.css"))
    });
}

#[test]
fn the_router_asks_the_browsers_own_records_of_a_sites_storage() {
    let mut router = crate::devtools::Router::default();
    let mut panel = Recorder::default();
    let mut page = Recorder {
        url: Some(Url::parse("https://a.test/").expect("a URL")),
        ..Recorder::default()
    };
    router.route(&mut panel, &mut page);
    panel.sent.clear();
    panel.out = vec![r#"{"id":1,"method":"storage.local"}"#.to_owned()];
    router.route(&mut panel, &mut page);
    // Not the page's to answer: the tab's storage is asked.
    assert!(page.asked.is_empty());
    assert_eq!(page.storage, [r#"{"id":1,"method":"storage.local"}"#]);
    assert_eq!(panel.sent, [r#"{"id":1,"result":[]}"#]);
}

#[test]
fn the_panels_storage_shows_what_the_site_keeps() {
    let server = serve(|path| match path {
        "/" => html("<title>Keeps</title><script>localStorage.setItem('theme', 'dark')</script>"),
        _ => None,
    });
    let mut window = Window::with_scripts(Some(server.url("/")));
    window.settle();
    window.press_key(Key::F12, NONE);
    window.run_until(|session| {
        panel_text(session, "storage-local").is_some_and(|text| text.contains("themedark"))
    });
}

/// M36.5's demo: a to-do list kept in IndexedDB, seeded from the server.
const TODO_IDB: &str = include_str!("../../../engine/tests/demos/todo-idb.html");

/// Runs `script` in the DevTools panel's own page, as the user's hand on
/// its controls: its value's preview.
fn in_panel(session: &mut Session, script: &str) -> devtools::Json {
    use devtools::Json;
    let panel = session.panel_mut().expect("the panel is open");
    let request = devtools::Request {
        id: 1,
        method: "console.evaluate".to_owned(),
        params: Json::object([("expression", Json::from(script))]),
    };
    panel.inspect(&request.write());
    let answer = panel.take_inspected().pop().expect("an answer");
    let answer = Json::parse(&answer).expect("JSON");
    answer
        .get("result")
        .and_then(|r| r.get("result"))
        .cloned()
        .unwrap_or_else(|| panic!("{answer:?}"))
}

/// Types `text` into the panel's element of id `id` and presses Enter.
fn type_in_panel(session: &mut Session, id: &str, text: &str) {
    let script = format!(
        "const at = document.getElementById({id:?}); at.focus(); at.value = {text:?}; \
         at.dispatchEvent(new KeyboardEvent('keydown', {{ key: 'Enter' }}))"
    );
    in_panel(session, &script);
}

/// Clicks the panel's element `selector`.
fn click_in_panel(session: &mut Session, selector: &str) {
    in_panel(
        session,
        &format!("document.querySelector({selector:?}).click()"),
    );
}

/// M36.5: the to-do list debugged through the panel alone. Its entries
/// come from the server (the request in the network log) and are kept
/// in IndexedDB (the records in the storage panel); completing one throws
/// (the error in the console, with its stack); a completed one is not
/// struck out (the element inspected, its rules read, its style edited);
/// and what was kept is there when the page is loaded again.
#[test]
fn the_to_do_list_is_debugged_through_the_panel() {
    let server = serve(|path| match path {
        "/" => html(TODO_IDB),
        "/seed.json" => Some(Reply::ok(
            "application/json",
            br#"["buy milk","write tests"]"#,
        )),
        _ => None,
    });
    let memory = profile::Memory::new();
    let mut window = Window::on_profile(Some(server.url("/")), &memory);
    window.run_until(|session| session.title().starts_with("todos: 2"));
    window.press_key(Key::F12, NONE);
    window.run_until(|session| {
        panel_text(session, "elements-tree").is_some_and(|tree| tree.contains("todoapp"))
    });
    // The bug, met as the user meets it: an entry completed, from the
    // console's prompt.
    click_in_panel(&mut window.session, "#tab-console");
    type_in_panel(
        &mut window.session,
        "console-prompt",
        "document.querySelector('.toggle').click()",
    );
    window.run_until(|session| {
        panel_console(session).contains("Uncaught ReferenceError: updat is not defined")
    });
    // Its place, the page's.
    let console = panel_console(&mut window.session);
    assert!(
        console.contains("todo-idb") || console.contains(server.url("/").as_str()),
        "{console}"
    );
    // The entry completed, inspected: the console's node revealed in the
    // element inspector, its rules read.
    type_in_panel(
        &mut window.session,
        "console-prompt",
        "document.querySelector('.completed label')",
    );
    window.run_until(|session| panel_console(session).contains("<label>"));
    click_in_panel(&mut window.session, "#console-lines .result .v-node");
    window.run_until(|session| {
        panel_text(session, "elements-styles")
            .is_some_and(|styles| styles.contains(".todo-list li label {"))
    });
    // No rule strikes it out: `li.complete` is not its class.
    let styles = panel_text(&mut window.session, "elements-styles").unwrap_or_default();
    assert!(!styles.contains("line-through"), "{styles}");
    // Edited: its style attribute strikes it out, as the page then shows.
    type_in_panel(
        &mut window.session,
        "elements-add-style",
        "text-decoration: line-through",
    );
    // (getComputedStyle's list has no text-decoration, so the computed
    // pane cannot show it: the label's own style and the tree do.)
    window.run_until(|session| {
        panel_text(session, "elements-styles")
            .is_some_and(|styles| styles.contains("element.style {text-decoration: line-through;"))
            && panel_text(session, "elements-tree")
                .is_some_and(|tree| tree.contains("style=\"text-decoration: line-through\""))
    });
    // The request that seeded it, in the network log.
    click_in_panel(&mut window.session, "#tab-network");
    window.run_until(|session| {
        panel_text(session, "network-list").is_some_and(|list| list.contains("seed.json"))
    });
    // Its records, in the storage panel.
    click_in_panel(&mut window.session, "#tab-storage");
    window.run_until(|session| {
        panel_text(session, "storage-idb").is_some_and(|idb| {
            idb.contains("todos (version 1)")
                && idb.contains("buy milk")
                && idb.contains("write tests")
        })
    });
    // Loaded again, the list is what was kept, the first entry done.
    window.session.flush_profile().expect("written");
    let mut again = Window::on_profile(Some(server.url("/")), &memory);
    again.run_until(|session| session.title().starts_with("todos: 2"));
    let requests = server.requests();
    let seeded = requests.iter().filter(|r| r.target == "/seed.json").count();
    assert_eq!(seeded, 1, "the second load reads what was kept");
}

/// The middle of tab `n` in the strip, as wide as tabs are while there
/// is room (M37).
fn tab_in_strip(n: usize) -> (f32, f32) {
    (6.0 + ui::tabs::TAB_WIDTH * n as f32 + 60.0, 18.0)
}

/// The new-tab button after `count` tabs.
fn new_tab_button(count: usize) -> (f32, f32) {
    (6.0 + ui::tabs::TAB_WIDTH * count as f32 + 3.0 + 14.0, 18.0)
}

/// The strip's titles, in its order.
fn titles(session: &Session) -> Vec<String> {
    session
        .tab_items()
        .into_iter()
        .map(|item| item.title)
        .collect()
}

/// Pages that say their own name, and whether they are seen.
fn named_pages() -> TestServer {
    serve(|path| {
        let name = path.trim_start_matches('/');
        html(&format!(
            "<title>{name}</title><script>document.addEventListener('visibilitychange', \
             () => document.title = '{name} ' + document.visibilityState)</script><p>{name}"
        ))
    })
}

#[test]
fn tabs_open_switch_and_close_by_the_keys() {
    let server = named_pages();
    let mut window = Window::with_scripts(Some(server.url("/a")));
    window.settle();
    assert_eq!(window.session.tab_count(), 1);
    // Ctrl+T: a new tab after the one shown, on the start page, the
    // field with the keyboard.
    window.chord("t", CTRL);
    assert_eq!(
        (window.session.tab_count(), window.session.active()),
        (2, 1)
    );
    assert_eq!(window.session.focus(), Focus::Field);
    assert_eq!(
        window.session.engine().url().map(Url::as_str),
        Some("about:silver")
    );
    window.type_text(server.url("/b").as_str());
    window.press_key(Key::Enter, NONE);
    window.settle();
    window.wait_until(|session| session.title().starts_with("b "));
    // The window's title is the tab shown's; the first tab, hidden, heard
    // it.
    window.wait_until(|session| titles(session)[0] == "a hidden");
    // Ctrl+Tab and Ctrl+Shift+Tab go round; Ctrl+1 and Ctrl+9 jump.
    window.press_key(Key::Tab, CTRL);
    assert_eq!(window.session.active(), 0);
    window.wait_until(|session| session.title().starts_with("a visible"));
    window.press_key(Key::Tab, CTRL);
    assert_eq!(window.session.active(), 1);
    window.press_key(Key::Tab, CTRL);
    assert_eq!(window.session.active(), 0, "round the strip");
    window.press_key(
        Key::Tab,
        Modifiers {
            shift: true,
            ..CTRL
        },
    );
    assert_eq!(window.session.active(), 1);
    window.chord("1", CTRL);
    assert_eq!(window.session.active(), 0);
    window.chord("9", CTRL);
    assert_eq!(window.session.active(), 1);
    window.press_key(Key::PageUp, CTRL);
    assert_eq!(window.session.active(), 0);
    // Ctrl+W closes the tab shown; the last one closes the window.
    window.chord("w", CTRL);
    assert_eq!(window.session.tab_count(), 1);
    assert!(window.session.title().starts_with("b "));
    assert!(!window.session.wants_close());
    window.press_key(Key::F4, CTRL);
    assert!(window.session.wants_close());
}

#[test]
fn the_strip_shows_the_tabs_and_takes_the_pointer() {
    let server = named_pages();
    let mut window = Window::with_scripts(Some(server.url("/a")));
    window.settle();
    window.wait_until(|session| titles(session) == ["a"]);
    // The new-tab button opens one, shown.
    let (x, y) = new_tab_button(1);
    window.click(x, y);
    assert_eq!(
        (window.session.tab_count(), window.session.active()),
        (2, 1)
    );
    // A tab pressed is shown.
    let (x, y) = tab_in_strip(0);
    window.click(x, y);
    assert_eq!(window.session.active(), 0);
    window.wait_until(|session| session.title().starts_with("a visible"));
    // The middle button closes the tab it is released on.
    let (x, y) = tab_in_strip(1);
    window.session.pointer_moved(x, y);
    window.session.press(Button::Middle, window.clock);
    window.session.release(Button::Middle);
    assert_eq!(window.session.tab_count(), 1);
    // The close button of the tab shown closes it, the window with it.
    let close = (6.0 + ui::tabs::TAB_WIDTH - 18.0, 18.0);
    window.click(close.0, close.1);
    assert!(window.session.wants_close());
}

#[test]
fn a_tab_dragged_along_the_strip_moves() {
    let server = named_pages();
    let mut window = Window::with_scripts(Some(server.url("/a")));
    window.settle();
    for name in ["b", "c"] {
        window.chord("t", CTRL);
        window.type_text(server.url(&format!("/{name}")).as_str());
        window.press_key(Key::Enter, NONE);
        window.settle();
    }
    let first_words = |session: &Session| -> Vec<String> {
        titles(session)
            .into_iter()
            .map(|title| title.split(' ').next().unwrap_or_default().to_owned())
            .collect()
    };
    window.wait_until(|session| first_words(session) == ["a", "b", "c"]);
    // The first tab, dragged two tabs' width right, is last.
    let (x, y) = tab_in_strip(0);
    window.session.pointer_moved(x, y);
    window.session.press(Button::Left, window.clock);
    assert_eq!(window.session.active(), 0, "a tab pressed is shown");
    for step in 1..=10 {
        let dx = ui::tabs::TAB_WIDTH * 2.0 * step as f32 / 10.0;
        window.session.pointer_moved(x + dx, y);
    }
    window.session.release(Button::Left);
    assert_eq!(first_words(&window.session), ["b", "c", "a"]);
    assert_eq!(window.session.active(), 2, "the tab shown moved with it");
}

#[test]
fn each_tab_keeps_its_own_edit_of_the_field() {
    let server = named_pages();
    let mut window = Window::with_scripts(Some(server.url("/a")));
    window.settle();
    window.chord("l", CTRL);
    window.type_text("half typed");
    window.chord("t", CTRL);
    assert_eq!(window.session.toolbar().text(), "about:silver");
    window.chord("1", CTRL);
    assert_eq!(window.session.toolbar().text(), "half typed");
    assert_eq!(window.session.focus(), Focus::Field);
}

#[test]
fn a_discarded_tab_is_loaded_again_when_shown() {
    let server = named_pages();
    let mut window = Window::with_scripts(Some(server.url("/a")));
    window.settle();
    window.chord("t", CTRL);
    window.session.discard(0);
    // It keeps its title in the strip, and loads its page again when
    // shown.
    assert!(titles(&window.session)[0].starts_with("a"));
    window.chord("1", CTRL);
    window.settle();
    assert_eq!(window.session.engine().url(), Some(&server.url("/a")));
    window.wait_until(|session| session.title().starts_with("a"));
    // A tab shown is never discarded.
    window.session.discard(0);
    assert_eq!(window.session.engine().url(), Some(&server.url("/a")));
}

#[test]
fn the_tab_shown_least_recently_is_discarded_first() {
    assert_eq!(
        least_recently_shown(&[Some(5), None, Some(2), Some(9)]),
        Some(2)
    );
    assert_eq!(least_recently_shown(&[None, None]), None);
}

#[test]
fn a_hidden_tabs_timers_and_loads_go_on() {
    let server = serve(|path| match path {
        "/timer" => html(
            "<title>waiting</title><script>setTimeout(() => document.title = 'timed', 400)</script>",
        ),
        _ => html("<title>loaded</title><p>loaded"),
    });
    let mut window = Window::with_scripts(Some(server.url("/timer")));
    window.settle();
    // A tab opened behind the one shown loads there.
    assert!(window.session.new_tab(Some(server.url("/other")), false));
    assert_eq!(window.session.active(), 0);
    window.wait_until(|session| titles(session)[1] == "loaded");
    // The tab shown put aside: its timer still runs, hidden.
    window.chord("t", CTRL);
    assert_eq!(window.session.active(), 1);
    assert_eq!(
        titles(&window.session)[0],
        "waiting",
        "put aside before its timer"
    );
    let deadline = Instant::now() + WAIT;
    while titles(&window.session)[0] != "timed" {
        assert!(Instant::now() < deadline, "the hidden tab's timer runs");
        window.session.tick();
        std::thread::sleep(Duration::from_millis(20));
    }
}
