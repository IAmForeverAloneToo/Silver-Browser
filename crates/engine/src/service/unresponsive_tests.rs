//! The page unresponsive bar's tab side (2026-11-26, as Chrome's hang
//! monitor): input a renderer has not said it took makes the page
//! unresponsive after [`UNRESPONSIVE_AFTER`]; input taken, a busy page
//! nobody touches and a hidden tab never do; "Wait" gives it the time
//! again and "Exit page" ends its renderer.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use net::test_server::{Reply, TestServer};

use super::{RemoteTab, ThreadLauncher, UNRESPONSIVE_AFTER};
use crate::view::View;
use crate::{Button, FrameTime, LoadError, LoadState};

const WAIT: Duration = Duration::from_secs(30);
const SIZE: (u32, u32) = (320, 240);

fn fonts() -> text::FontCollection {
    text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
}

/// A tab of thread renderers whose clock runs `ahead` milliseconds past
/// real time, which a test moves on.
fn tab(ahead: &Arc<AtomicU64>) -> RemoteTab {
    let launcher = Arc::new(ThreadLauncher::with_fonts(fonts()));
    let mut tab = RemoteTab::new(launcher, fonts());
    tab.set_viewport(SIZE.0, SIZE.1);
    tab.set_scripting(true);
    let began = Instant::now();
    let ahead = Arc::clone(ahead);
    tab.set_clock(Arc::new(move || {
        began.elapsed() + Duration::from_millis(ahead.load(Ordering::SeqCst))
    }));
    tab
}

/// Takes what the renderer says until `done` holds of the tab.
fn settle(tab: &mut RemoteTab, what: &str, mut done: impl FnMut(&mut RemoteTab) -> bool) {
    let began = Instant::now();
    loop {
        tab.poll_network();
        tab.run_tasks();
        if done(tab) {
            return;
        }
        assert!(began.elapsed() < WAIT, "waited {WAIT:?} for {what}");
        thread::sleep(Duration::from_millis(2));
    }
}

/// Takes what the renderer says for `long`.
fn pump(tab: &mut RemoteTab, long: Duration) {
    let began = Instant::now();
    while began.elapsed() < long {
        tab.poll_network();
        tab.run_tasks();
        thread::sleep(Duration::from_millis(2));
    }
}

/// A page whose press keeps its renderer's loop for 1.5 s, under the
/// scripts' step budget: input sent meanwhile goes unanswered.
const BUSY_ON_PRESS: &str = "<body style='height: 2000px'><script>\
    addEventListener('pointerdown', () => { \
      const end = performance.now() + 1500; while (performance.now() < end) {} });\
    </script>";

fn loaded(page: &'static str, ahead: &Arc<AtomicU64>) -> (RemoteTab, TestServer) {
    let server = TestServer::plain(move |_| Reply::ok("text/html", page.as_bytes()));
    let mut tab = tab(ahead);
    tab.frame_at(SIZE.0, SIZE.1, FrameTime::at(Duration::ZERO));
    tab.navigate(server.url("/"));
    settle(&mut tab, "the page loaded", |tab| {
        *tab.load_state() == LoadState::Loaded
    });
    settle(&mut tab, "the page's first frame", |tab| tab.needs_frame());
    tab.frame_at(SIZE.0, SIZE.1, FrameTime::at(Duration::ZERO));
    (tab, server)
}

/// The clock moved on by `UNRESPONSIVE_AFTER` and a second more.
fn later(ahead: &Arc<AtomicU64>) {
    ahead.fetch_add(
        UNRESPONSIVE_AFTER.as_millis() as u64 + 1000,
        Ordering::SeqCst,
    );
}

#[test]
fn input_the_renderer_took_never_makes_the_page_unresponsive() {
    let ahead = Arc::new(AtomicU64::new(0));
    let (mut tab, _server) = loaded("<body style='height: 2000px'>still</body>", &ahead);
    tab.wheel(10.0, 10.0, 40.0);
    pump(&mut tab, Duration::from_millis(300));
    later(&ahead);
    assert!(!tab.unresponsive(), "input taken in time");
}

#[test]
fn input_left_unanswered_makes_the_page_unresponsive_until_it_is_taken() {
    let ahead = Arc::new(AtomicU64::new(0));
    let (mut tab, _server) = loaded(BUSY_ON_PRESS, &ahead);
    tab.pointer_down(10.0, 10.0, Button::Primary, 1);
    pump(&mut tab, Duration::from_millis(150));
    // The renderer is in the press's handler: the wheel waits.
    tab.wheel(10.0, 10.0, 40.0);
    pump(&mut tab, Duration::from_millis(150));
    assert!(!tab.unresponsive(), "not before its time");
    later(&ahead);
    assert!(tab.unresponsive(), "input unanswered past its time");
    settle(&mut tab, "the busy renderer to take the wheel", |tab| {
        !tab.unresponsive()
    });
}

#[test]
fn a_busy_page_nobody_touches_is_never_unresponsive() {
    let ahead = Arc::new(AtomicU64::new(0));
    let (mut tab, _server) = loaded(
        "<script>setTimeout(() => { const end = performance.now() + 1500; \
           while (performance.now() < end) {} }, 0);</script>",
        &ahead,
    );
    pump(&mut tab, Duration::from_millis(300));
    later(&ahead);
    assert!(!tab.unresponsive(), "no input, no bar, as Chrome");
}

#[test]
fn a_hidden_tab_waits_wait_gives_it_time_and_exit_ends_its_renderer() {
    let ahead = Arc::new(AtomicU64::new(0));
    let (mut tab, _server) = loaded(BUSY_ON_PRESS, &ahead);
    tab.pointer_down(10.0, 10.0, Button::Primary, 1);
    pump(&mut tab, Duration::from_millis(150));
    tab.wheel(10.0, 10.0, 40.0);
    later(&ahead);
    assert!(tab.unresponsive());
    // Hidden, never; shown again, its time starts again.
    tab.set_hidden(true);
    assert!(!tab.unresponsive(), "a hidden tab is never unresponsive");
    tab.set_hidden(false);
    assert!(!tab.unresponsive(), "shown again, its time starts again");
    later(&ahead);
    assert!(tab.unresponsive());
    // "Wait": its time again.
    tab.wait_unresponsive();
    assert!(!tab.unresponsive(), "Wait gives it its time again");
    later(&ahead);
    assert!(tab.unresponsive());
    // "Exit page": its renderer ends, as a crash ends it.
    tab.exit_unresponsive();
    assert!(!tab.unresponsive());
    settle(&mut tab, "the page's renderer gone", |tab| {
        matches!(tab.load_state(), LoadState::Failed(LoadError::Lost))
    });
}
