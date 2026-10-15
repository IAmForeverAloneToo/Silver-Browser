//! A tab whose page a renderer shows (M34.5): the same pixels as one
//! engine's, input and its answers, the console, the cookies kept in the
//! browser, and a renderer gone.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use net::test_server::{Reply, TestServer};

use super::{Launched, Launcher, RemoteTab, RendererProcess, ThreadLauncher};
use crate::ipc::{self, ToBrowser, ToRenderer};
use crate::view::View;
use crate::{Button, Engine, FrameTime, LoadError, LoadState, Url};

const WAIT: Duration = Duration::from_secs(30);
const SIZE: (u32, u32) = (320, 240);

fn fonts() -> text::FontCollection {
    text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
}

fn tab() -> RemoteTab {
    let launcher = Arc::new(ThreadLauncher::with_fonts(fonts()));
    let mut tab = RemoteTab::new(launcher, fonts());
    tab.set_viewport(SIZE.0, SIZE.1);
    let began = Instant::now();
    tab.set_clock(Arc::new(move || began.elapsed()));
    tab
}

/// Takes what the renderer says until `done` holds of the tab.
fn settle(tab: &mut RemoteTab, mut done: impl FnMut(&mut RemoteTab) -> bool) {
    let began = Instant::now();
    loop {
        tab.poll_network();
        tab.run_tasks();
        if done(tab) {
            return;
        }
        assert!(began.elapsed() < WAIT, "the tab gets there in time");
        thread::sleep(Duration::from_millis(2));
    }
}

/// The tab's page loaded, and its last frame at the tab's size once the
/// renderer has drawn no other for a while.
fn shown(tab: &mut RemoteTab) -> Vec<u8> {
    tab.frame_at(SIZE.0, SIZE.1, FrameTime::at(Duration::ZERO));
    settle(tab, |tab| *tab.load_state() == LoadState::Loaded);
    settle(tab, |tab| tab.needs_frame());
    let mut last = Instant::now();
    let mut frame = Vec::new();
    while last.elapsed() < Duration::from_millis(300) {
        tab.poll_network();
        if tab.needs_frame() || frame.is_empty() {
            frame = tab
                .frame_at(SIZE.0, SIZE.1, FrameTime::at(Duration::ZERO))
                .to_vec();
            last = Instant::now();
        }
        thread::sleep(Duration::from_millis(5));
    }
    frame
}

fn serve(page: &'static str) -> TestServer {
    TestServer::plain(move |_| Reply::ok("text/html", page.as_bytes()))
}

#[test]
fn a_renderers_page_is_drawn_as_one_engine_draws_it() {
    let server = serve(
        "<body style='margin: 10px; background: #fed'>\
         <h1 style='color: navy'>Hello</h1><p>A renderer drew this.</p>\
         <div style='width: 50px; height: 30px; background: teal'></div>",
    );
    let mut remote = tab();
    remote.navigate(server.url("/"));
    let pixels = shown(&mut remote);

    let mut engine = Engine::with_fonts(fonts());
    engine.set_viewport(SIZE.0, SIZE.1);
    engine.navigate(server.url("/"));
    let began = Instant::now();
    while *engine.load_state() != LoadState::Loaded {
        engine.poll_network();
        assert!(began.elapsed() < WAIT);
        thread::sleep(Duration::from_millis(2));
    }
    let own = engine.frame_at(SIZE.0, SIZE.1, FrameTime::at(Duration::ZERO));
    assert_eq!(pixels.len(), own.len());
    assert!(pixels == own, "the same pixels");
    assert_eq!(remote.url(), Some(&server.url("/")));
    assert!(!remote.can_go_back());
}

#[test]
fn input_reaches_the_page_and_its_answers_come_back() {
    let server = serve(
        "<title>start</title><body style='margin: 0'>\
         <div id=d style='width: 100px; height: 100px'>x</div>\
         <script>document.getElementById('d').onclick = (e) => { document.title = 'clicked'; \
         console.log('click at', e.clientX) };</script>",
    );
    let mut tab = tab();
    tab.set_scripting(true);
    tab.navigate(server.url("/"));
    shown(&mut tab);
    assert_eq!(tab.title(), "start");
    tab.pointer_move(20.0, 20.0);
    tab.pointer_down(20.0, 20.0, Button::Primary, 1);
    tab.pointer_up(20.0, 20.0, Button::Primary, 1);
    settle(&mut tab, |tab| tab.title() == "clicked");
    let console: Vec<String> = tab.take_console().into_iter().map(|e| e.text).collect();
    assert_eq!(console, ["click at 20"]);
}

#[test]
fn the_keyboard_leaving_the_page_is_said_after_the_key() {
    let server = serve(
        "<input id=i>\
         <script>document.getElementById('i').focus();</script>",
    );
    let mut tab = tab();
    tab.set_scripting(true);
    tab.set_window_focus(true);
    tab.navigate(server.url("/"));
    shown(&mut tab);
    settle(&mut tab, |tab| tab.can_paste());
    assert!(!tab.take_focus_left());
    let tab_key = crate::Key::new("Tab", "Tab");
    assert_eq!(tab.key_down(&tab_key), crate::KeyOutcome::Taken, "for now");
    settle(&mut tab, |tab| tab.take_focus_left());
}

#[test]
fn a_selection_is_copied_when_the_renderer_answers() {
    let server = serve(
        "<input id=i value='copy me'>\
         <script>const i = document.getElementById('i'); i.focus(); i.select();</script>",
    );
    let mut tab = tab();
    tab.set_scripting(true);
    tab.set_window_focus(true);
    tab.navigate(server.url("/"));
    shown(&mut tab);
    settle(&mut tab, |tab| tab.can_paste());
    assert_eq!(tab.copy(), None, "the answer comes later");
    let mut copied = None;
    settle(&mut tab, |tab| {
        copied = tab.take_copied();
        copied.is_some()
    });
    assert_eq!(copied, Some(("copy me".to_owned(), false)));
}

#[test]
fn cookies_and_storage_stay_in_the_browser() {
    let server = TestServer::plain(|_| {
        Reply::with_fields(
            200,
            &[("Content-Type", "text/html"), ("Set-Cookie", "a=1")],
            b"<script>document.cookie = 'b=2'; localStorage.k = 'v'; \
              console.log(document.cookie, localStorage.k)</script>",
        )
    });
    let mut tab = tab();
    tab.set_scripting(true);
    tab.navigate(server.url("/"));
    shown(&mut tab);
    let mut lines = Vec::new();
    settle(&mut tab, |tab| {
        lines.extend(tab.take_console().into_iter().map(|e| e.text));
        !lines.is_empty()
    });
    assert_eq!(lines, ["a=1; b=2 v"]);
}

#[test]
fn a_renderer_gone_leaves_the_page_saying_so() {
    let server = serve("<p>here</p>");
    let mut tab = tab();
    tab.navigate(server.url("/"));
    shown(&mut tab);
    tab.kill_renderer();
    settle(&mut tab, |tab| {
        *tab.load_state() == LoadState::Failed(LoadError::Lost)
    });
    assert!(
        tab.take_notice()
            .is_some_and(|notice| notice.contains("renderer"))
    );
}

#[test]
fn a_document_of_another_site_navigated_to_is_the_renderers_from_its_response() {
    let server = TestServer::plain(|request| {
        let port = request
            .header("Host")
            .and_then(|host| host.rsplit(':').next())
            .unwrap_or_default()
            .to_owned();
        match request.target.as_str() {
            "/a" => Reply::ok(
                "text/html",
                format!("<script>location.href = 'http://b.localhost:{port}/b'</script>").as_bytes(),
            ),
            "/b" => Reply::ok(
                "text/html",
                b"<script>fetch('/data').then(r => r.text()).then(t => console.log('b got', t))</script>",
            ),
            _ => Reply::ok("text/plain", b"data"),
        }
    });
    let mut tab = tab();
    tab.set_scripting(true);
    let a = Url::parse(&format!("http://a.localhost:{}/a", server.port())).expect("a URL");
    tab.navigate(a);
    let mut lines = Vec::new();
    settle(&mut tab, |tab| {
        lines.extend(tab.take_console().into_iter().map(|entry| entry.text));
        !lines.is_empty()
    });
    assert_eq!(lines, ["b got data"]);
    assert_eq!(tab.load_state(), &LoadState::Loaded);
}

/// A launcher whose renderer answers the hello and then sends what
/// `lie` makes of the view's frame number.
struct Lying(fn() -> Vec<ToBrowser>);

impl Launcher for Lying {
    fn launch(&self, _: u32) -> std::io::Result<Launched> {
        let ((browser_in, browser_out), (mut renderer_in, renderer_out)) = ipc::duplex();
        let lies = self.0;
        let breakers = [browser_out.breaker(), renderer_out.breaker()];
        thread::spawn(move || {
            let sender = ipc::Sender::<ToBrowser>::new(renderer_out);
            let _ = ipc::read_message::<ToRenderer>(&mut renderer_in);
            let _ = sender.send(&ToBrowser::Hello {
                version: ipc::IPC_VERSION,
            });
            // It lies once given its view, as a renderer would: the
            // browser has given it its site and frames by then.
            while let Ok(Some(message)) = ipc::read_message::<ToRenderer>(&mut renderer_in) {
                if matches!(message, ToRenderer::CreateView { .. }) {
                    break;
                }
            }
            for lie in lies() {
                let _ = sender.send(&lie);
            }
            // Read until the browser ends it.
            while let Ok(Some(_)) = ipc::read_message::<ToRenderer>(&mut renderer_in) {}
        });
        Ok(Launched {
            from_renderer: Box::new(browser_in),
            to_renderer: Box::new(browser_out),
            log: None,
            process: Box::new(Broken(breakers)),
        })
    }
}

struct Broken([ipc::PipeBreaker; 2]);

impl RendererProcess for Broken {
    fn kill(&mut self) {
        for breaker in &self.0 {
            breaker.break_pipe();
        }
    }
    fn id(&self) -> u32 {
        0
    }
    fn exit_code(&mut self) -> Option<i64> {
        None
    }
}

/// What the tab says once a renderer has told it `lies`.
fn lied_to(lies: fn() -> Vec<ToBrowser>) -> String {
    let mut tab = RemoteTab::new(Arc::new(Lying(lies)), fonts());
    tab.navigate(Url::parse("about:blank").expect("a URL"));
    settle(&mut tab, |tab| {
        *tab.load_state() == LoadState::Failed(LoadError::Lost)
    });
    tab.take_notice().unwrap_or_default()
}

#[test]
fn a_renderer_that_lies_about_its_frames_is_killed() {
    let past = lied_to(|| {
        vec![ToBrowser::Pixels {
            view: 1,
            frame: 1,
            width: 4,
            height: 4,
            first_row: 3,
            bytes: ipc::Bytes(vec![0; 32]),
        }]
    });
    assert!(past.contains("pixels past the frame"), "{past}");
    let huge = lied_to(|| {
        vec![ToBrowser::Frame {
            view: 1,
            frame: 1,
            width: 100_000,
            height: 100_000,
        }]
    });
    assert!(huge.contains("a frame of 100000 by 100000"), "{huge}");
    let other = lied_to(|| {
        vec![ToBrowser::Notice {
            view: 2,
            text: "not mine".into(),
        }]
    });
    assert!(other.contains("view 2, not the renderer's"), "{other}");
}

#[test]
fn a_renderer_draws_no_frame_while_the_browser_has_not_taken_the_last() {
    let server = serve(
        "<style>@keyframes k { from { margin-left: 0 } to { margin-left: 200px } }          div { width: 20px; height: 20px; background: red;          animation: k 1s linear infinite }</style><div></div>",
    );
    let mut tab = tab();
    tab.navigate(server.url("/"));
    tab.frame_at(SIZE.0, SIZE.1, FrameTime::at(Duration::ZERO));
    settle(&mut tab, |tab| {
        *tab.load_state() == LoadState::Loaded && tab.needs_frame()
    });
    let before = tab.frames_received();
    // The browser takes nothing for a while: the renderer waits.
    thread::sleep(Duration::from_millis(500));
    tab.poll_network();
    let came = tab.frames_received() - before;
    assert!(came <= 1, "{came} frames came untaken");
    // Taken, the next come.
    settle(&mut tab, |tab| tab.frames_received() >= before + 3);
}

#[test]
fn a_renderer_that_lies_about_frames_is_killed() {
    let forged = lied_to(|| {
        vec![ToBrowser::PostFrame {
            from: None,
            to: 1,
            task: ipc::WireTask::Message {
                data: "x".into(),
                origin: "https://victim.test".into(),
                target_origin: "*".into(),
            },
        }]
    });
    assert!(
        forged.contains("a message from https://victim.test"),
        "{forged}"
    );
    let unknown = lied_to(|| {
        vec![ToBrowser::NewFrame {
            call: 1,
            parent: 999,
        }]
    });
    assert!(unknown.contains("frame 999 is not renderer"), "{unknown}");
    let dropped = lied_to(|| vec![ToBrowser::FrameDropped { key: 999 }]);
    assert!(
        dropped.contains("frame 999, not of the renderer's pages"),
        "{dropped}"
    );
    let policy = lied_to(|| {
        vec![ToBrowser::FramePolicy {
            key: 999,
            sandbox: 0,
        }]
    });
    assert!(
        policy.contains("frame 999, not of the renderer's pages"),
        "{policy}"
    );
}
