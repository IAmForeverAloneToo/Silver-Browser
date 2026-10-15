//! A renderer that stops (M34.4, D34): the page it showed says so, the
//! browser's own page, until a reload brings it back in a new renderer;
//! a frame it showed says so in its hole, its parent's `load` not held
//! by it, and comes back when its parent navigates it; the frames of a
//! page it showed go with it.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use net::test_server::{Reply, TestServer};

use super::{RemoteTab, ThreadLauncher};
use crate::ipc;
use crate::view::View;
use crate::{Button, FrameTime, LoadError, LoadState, Url};

const WAIT: Duration = Duration::from_secs(30);
const SIZE: (u32, u32) = (400, 300);

/// The background of the browser's pages: the stopped page's.
const STOPPED: [u8; 4] = [246, 246, 244, 255];
const GREEN: [u8; 4] = [0, 128, 0, 255];
const PARENT: [u8; 4] = [255, 238, 221, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

fn fonts() -> text::FontCollection {
    text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
}

fn tab() -> RemoteTab {
    let launcher = Arc::new(ThreadLauncher::with_fonts(fonts()));
    let mut tab = RemoteTab::with_cap(launcher, fonts(), 16, false);
    tab.set_viewport(SIZE.0, SIZE.1);
    tab.set_scripting(true);
    let began = Instant::now();
    tab.set_clock(Arc::new(move || began.elapsed()));
    tab.frame_at(SIZE.0, SIZE.1, FrameTime::at(Duration::ZERO));
    tab
}

/// Takes what the renderers say until `done` holds of the tab, the
/// console's lines gathered in `lines`.
fn settle(
    tab: &mut RemoteTab,
    lines: &mut Vec<String>,
    what: &str,
    mut done: impl FnMut(&mut RemoteTab, &[String]) -> bool,
) {
    let began = Instant::now();
    loop {
        tab.poll_network();
        tab.run_tasks();
        lines.extend(tab.take_console().into_iter().map(|entry| entry.text));
        if done(tab, lines) {
            return;
        }
        assert!(
            began.elapsed() < WAIT,
            "{what}: {lines:?} {:?} {:?} {:?}",
            tab.load_state(),
            tab.renderers(),
            tab.frame_roots()
        );
        thread::sleep(Duration::from_millis(2));
    }
}

/// The tab's composited frame once it stops changing.
fn composed(tab: &mut RemoteTab) -> Vec<u8> {
    let mut frame = Vec::new();
    let mut last = Instant::now();
    let began = Instant::now();
    while last.elapsed() < Duration::from_millis(400) && began.elapsed() < WAIT {
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

fn pixel(frame: &[u8], x: u32, y: u32) -> [u8; 4] {
    let at = ((y * SIZE.0 + x) * 4) as usize;
    frame[at..at + 4].try_into().expect("a pixel")
}

fn server() -> TestServer {
    TestServer::plain(|request| {
        let host = request.header("Host").unwrap_or_default().to_owned();
        let port = host.rsplit(':').next().unwrap_or_default().to_owned();
        let frame = |src: &str| {
            format!(
                "<body style='margin: 0; background: #fed'>\
                 <iframe id=f src='http://b.localhost:{port}{src}' \
                 style='border: 0; margin: 20px; width: 200px; height: 100px'></iframe>\
                 <div id=o style='position: absolute; left: 150px; top: 60px; width: 40px; \
                 height: 30px; background: rgb(0, 0, 255)'></div>\
                 <script>document.getElementById('f').onload = () => console.log('framed load'); \
                 document.getElementById('o').onclick = () => \
                 document.getElementById('f').src = 'http://a.localhost:{port}/green?again';\
                 </script>"
            )
        };
        let body = match request.target.as_str() {
            "/framed" => frame("/green"),
            "/waits" => frame("/never"),
            "/never" => "<img src='/hang'>".to_owned(),
            "/hang" => return Reply::Hang,
            _ => "<body style='margin: 0; background: rgb(0, 128, 0)'>green".to_owned(),
        };
        Reply::ok("text/html", body.as_bytes())
    })
}

fn url(server: &TestServer, host: &str, path: &str) -> Url {
    Url::parse(&format!("http://{host}:{}{path}", server.port())).expect("a URL")
}

/// The renderer showing the tab's frame of `path`.
fn frame_renderer(tab: &RemoteTab, path: &str) -> Option<u32> {
    tab.frame_roots().into_iter().find_map(|(renderer, url)| {
        url.is_some_and(|url| url.as_str().ends_with(path))
            .then_some(renderer)
    })
}

#[test]
fn a_page_whose_renderer_stopped_says_so_until_reloaded() {
    let server = server();
    let mut tab = tab();
    let mut lines = Vec::new();
    tab.navigate(url(&server, "a.localhost", "/green"));
    settle(&mut tab, &mut lines, "the page", |tab, _| {
        *tab.load_state() == LoadState::Loaded
    });
    assert_eq!(pixel(&composed(&mut tab), 5, 5), GREEN);
    let first = tab.shown_renderer();

    tab.kill_renderer();
    settle(&mut tab, &mut lines, "the renderer gone", |tab, _| {
        *tab.load_state() == LoadState::Failed(LoadError::Lost)
    });
    assert!(tab.needs_frame());
    assert_eq!(
        pixel(&composed(&mut tab), 5, 5),
        STOPPED,
        "the browser's page"
    );
    assert!(
        tab.take_notice()
            .is_some_and(|notice| notice.contains("stopped")),
    );

    tab.reload();
    settle(&mut tab, &mut lines, "the page again", |tab, _| {
        *tab.load_state() == LoadState::Loaded
    });
    assert_eq!(pixel(&composed(&mut tab), 5, 5), GREEN);
    assert_ne!(tab.shown_renderer(), first, "a new renderer");
}

#[test]
fn a_frame_whose_renderer_stopped_says_so_and_comes_back_when_navigated() {
    let server = server();
    let mut tab = tab();
    let mut lines = Vec::new();
    tab.navigate(url(&server, "a.localhost", "/framed"));
    settle(&mut tab, &mut lines, "the frame", |tab, lines| {
        *tab.load_state() == LoadState::Loaded && lines.iter().any(|line| line == "framed load")
    });
    let frame = composed(&mut tab);
    assert_eq!(pixel(&frame, 200, 110), GREEN);
    let framing = frame_renderer(&tab, "/green").expect("the frame's renderer");

    tab.kill(framing);
    settle(
        &mut tab,
        &mut lines,
        "the frame's renderer gone",
        |tab, _| tab.frame_roots().is_empty(),
    );
    let frame = composed(&mut tab);
    assert_eq!(pixel(&frame, 200, 110), STOPPED, "the page saying so");
    assert_eq!(pixel(&frame, 5, 5), PARENT, "the parent's");
    assert_eq!(pixel(&frame, 160, 70), BLUE, "the parent's over the frame");
    assert_eq!(*tab.load_state(), LoadState::Loaded, "the page is not");

    // Navigated by its parent, even to a document of the parent's site,
    // it is shown again, by another renderer than the one that stopped.
    tab.pointer_move(160.0, 70.0);
    tab.pointer_down(160.0, 70.0, Button::Primary, 1);
    tab.pointer_up(160.0, 70.0, Button::Primary, 1);
    settle(&mut tab, &mut lines, "the frame again", |tab, _| {
        frame_renderer(tab, "/green?again").is_some()
    });
    assert_ne!(frame_renderer(&tab, "/green?again"), Some(framing));
    let frame = composed(&mut tab);
    assert_eq!(pixel(&frame, 200, 110), GREEN);
}

#[test]
fn a_frame_whose_renderer_stopped_holds_its_parents_load_no_more() {
    let server = server();
    let mut tab = tab();
    let mut lines = Vec::new();
    tab.navigate(url(&server, "a.localhost", "/waits"));
    settle(&mut tab, &mut lines, "the frame loading", |tab, _| {
        frame_renderer(tab, "/never").is_some()
    });
    thread::sleep(Duration::from_millis(200));
    tab.poll_network();
    let state = tab.load_state().clone();
    assert_eq!(
        state,
        LoadState::Loading,
        "{lines:?} {:?} {:?} {:?} {:?}",
        tab.frame_loads(),
        tab.renderers(),
        tab.told_loaded().to_vec(),
        tab.take_notice()
    );
    let framing = frame_renderer(&tab, "/never").expect("the frame's renderer");
    tab.kill(framing);
    settle(&mut tab, &mut lines, "the page loaded", |tab, lines| {
        *tab.load_state() == LoadState::Loaded && lines.iter().any(|line| line == "framed load")
    });
}

#[test]
fn the_frames_of_a_page_whose_renderer_stopped_go_with_it() {
    let server = server();
    let mut tab = tab();
    let mut lines = Vec::new();
    tab.navigate(url(&server, "a.localhost", "/framed"));
    settle(&mut tab, &mut lines, "the frame", |tab, _| {
        frame_renderer(tab, "/green").is_some()
    });
    tab.kill_renderer();
    settle(
        &mut tab,
        &mut lines,
        "the page and its frame gone",
        |tab, _| {
            *tab.load_state() == LoadState::Failed(LoadError::Lost) && tab.frame_roots().is_empty()
        },
    );
}

/// A launcher whose renderer answers the hello, takes its first message
/// and ends, as a crash would, the browser having written its dump.
struct Crashing;

impl super::Launcher for Crashing {
    fn launch(&self, _: u32) -> std::io::Result<super::Launched> {
        let ((browser_in, browser_out), (mut renderer_in, renderer_out)) = ipc::duplex();
        let breakers = [browser_out.breaker(), renderer_out.breaker()];
        thread::spawn(move || {
            let sender = ipc::Sender::<ipc::ToBrowser>::new(renderer_out);
            let _ = ipc::read_message::<ipc::ToRenderer>(&mut renderer_in);
            let _ = sender.send(&ipc::ToBrowser::Hello {
                version: ipc::IPC_VERSION,
            });
            let _ = ipc::read_message::<ipc::ToRenderer>(&mut renderer_in);
        });
        Ok(super::Launched {
            from_renderer: Box::new(browser_in),
            to_renderer: Box::new(browser_out),
            log: None,
            process: Box::new(Dumped(breakers)),
        })
    }
}

struct Dumped([ipc::PipeBreaker; 2]);

impl super::RendererProcess for Dumped {
    fn kill(&mut self) {
        for breaker in &self.0 {
            breaker.break_pipe();
        }
    }
    fn id(&self) -> u32 {
        0
    }
    fn exit_code(&mut self) -> Option<i64> {
        Some(0xC000_0005)
    }
    fn dump(&mut self) -> Option<std::path::PathBuf> {
        Some(std::path::PathBuf::from("Crashes").join("renderer-1.dmp"))
    }
}

#[test]
fn the_page_of_a_crashed_renderer_names_its_dump() {
    let mut tab = RemoteTab::new(Arc::new(Crashing), fonts());
    tab.set_viewport(SIZE.0, SIZE.1);
    tab.navigate(Url::parse("about:blank").expect("a URL"));
    let mut lines = Vec::new();
    settle(&mut tab, &mut lines, "the renderer gone", |tab, _| {
        *tab.load_state() == LoadState::Failed(LoadError::Lost)
    });
    let notice = tab.take_notice().unwrap_or_default();
    assert!(notice.contains("exit code 0xc0000005"), "{notice}");
    assert!(notice.contains("renderer-1.dmp"), "{notice}");
    assert_eq!(pixel(&composed(&mut tab), 5, 5), STOPPED);
}
