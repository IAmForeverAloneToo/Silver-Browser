//! Frames across processes (M34.1, D33): a frame of another site shown by
//! that site's renderer, its pixels in its parent's hole as one engine
//! draws them, the pointer's events in it its own, its load its parent's
//! `load`, messages both ways with the browser's word on their origin, its
//! sandboxing kept, and its view gone with its element.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use net::test_server::{Reply, TestServer};

use super::{RemoteTab, ThreadLauncher};
use crate::view::View;
use crate::{Button, Engine, FrameTime, LoadState, Url};

const WAIT: Duration = Duration::from_secs(30);
const SIZE: (u32, u32) = (400, 300);

fn fonts() -> text::FontCollection {
    text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
}

fn tab() -> RemoteTab {
    let launcher = Arc::new(ThreadLauncher::with_fonts(fonts()));
    let mut tab = RemoteTab::with_cap(launcher, fonts(), 16, false);
    tab.set_viewport(SIZE.0, SIZE.1);
    tab.set_scripting(true);
    tab.set_window_focus(true);
    let began = Instant::now();
    tab.set_clock(Arc::new(move || began.elapsed()));
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
            "{what}: {lines:?} {:?} {:?}",
            tab.renderers(),
            tab.frame_roots()
        );
        thread::sleep(Duration::from_millis(2));
    }
}

/// A server of `a.localhost` pages framing `b.localhost` ones.
fn server() -> TestServer {
    TestServer::plain(|request| {
        let host = request.header("Host").unwrap_or_default().to_owned();
        let port = host.rsplit(':').next().unwrap_or_default().to_owned();
        let body = match request.target.as_str() {
            "/parent" => format!(
                "<body style='margin: 0; background: #fed'>\
                 <iframe id=f src='http://b.localhost:{port}/child' \
                 style='border: 0; margin: 20px; width: 200px; height: 100px'></iframe>\
                 <div id=o style='position: absolute; left: 150px; top: 60px; width: 40px; \
                 height: 30px; background: rgb(0, 0, 255)'></div>\
                 <script>document.getElementById('o').onclick = () => console.log('overlay click'); \
                 document.getElementById('f').onload = () => console.log('framed load'); \
                 addEventListener('message', e => {{ console.log('parent got', e.data, e.origin); \
                 e.source.postMessage('back', '*'); }});</script>"
            ),
            "/child" => "<body style='margin: 0; background: rgb(0, 128, 0)'>\
                 <div id=d style='height: 100px'></div>\
                 <script>console.log('child ran'); \
                 document.getElementById('d').onclick = e => console.log('child click', e.clientX, e.clientY); \
                 addEventListener('message', e => console.log('child got', e.data, e.origin)); \
                 parent.postMessage('hello', '*');</script>"
                .to_owned(),
            "/sandboxed" => format!(
                "<iframe sandbox src='http://b.localhost:{port}/child'></iframe>"
            ),
            "/plain" => format!(
                "<body style='margin: 0; background: #fed'>\
                 <iframe src='http://b.localhost:{port}/still' \
                 style='border: 0; margin: 20px; width: 200px; height: 100px'></iframe>\
                 <div style='position: absolute; left: 150px; top: 60px; width: 40px; \
                 height: 30px; background: rgb(0, 0, 255)'></div>"
            ),
            "/still" => "<body style='margin: 0; background: rgb(0, 128, 0)'>\
                 <p style='margin: 10px'>framed</p>"
                .to_owned(),
            "/removed" => format!(
                "<iframe id=f src='http://b.localhost:{port}/still'></iframe>\
                 <script>document.getElementById('f').onload = () => \
                 setTimeout(() => {{ document.getElementById('f').remove(); \
                 console.log('removed') }}, 100);</script>"
            ),
            "/back" => format!(
                "<iframe id=f src='http://b.localhost:{port}/still'></iframe>\
                 <script>let once = false; document.getElementById('f').onload = () => {{ \
                 if (once) {{ return; }} once = true; \
                 document.getElementById('f').src = '/again'; }};</script>"
            ),
            "/again" => "<p>again</p>".to_owned(),
            "/counted" => format!(
                "<iframe id=f src='/same'></iframe><img src='/slow'>\
                 <script>let got = 0; addEventListener('message', () => got++); \
                 let once = false; document.getElementById('f').onload = () => {{ \
                 if (once) {{ return; }} once = true; \
                 document.getElementById('f').src = 'http://b.localhost:{port}/child'; }}; \
                 onload = () => console.log('load ' + got);</script>"
            ),
            "/same" => "<p>same</p>".to_owned(),
            "/slow" => {
                return Reply::After(
                    Duration::from_millis(1000),
                    Box::new(Reply::ok("text/plain", b"not an image")),
                );
            }
            _ => String::new(),
        };
        Reply::ok("text/html", body.as_bytes())
    })
}

fn url(server: &TestServer, host: &str, path: &str) -> Url {
    Url::parse(&format!("http://{host}:{}{path}", server.port())).expect("a URL")
}

/// The tab's composited frame once it has drawn the page and its frame.
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

#[test]
fn a_frame_of_another_site_is_its_sites_renderers_and_drawn_in_its_hole() {
    let server = server();
    let mut tab = tab();
    tab.frame_at(SIZE.0, SIZE.1, FrameTime::at(Duration::ZERO));
    tab.navigate(url(&server, "a.localhost", "/plain"));
    let mut lines = Vec::new();
    settle(&mut tab, &mut lines, "the frame shown", |tab, _| {
        *tab.load_state() == LoadState::Loaded
            && tab.frame_roots().iter().any(|(_, url)| {
                url.as_ref()
                    .is_some_and(|url| url.as_str().ends_with("/still"))
            })
    });
    let renderers = tab.renderers();
    assert_eq!(renderers.len(), 2, "{renderers:?}");
    let frame = composed(&mut tab);
    assert_eq!(pixel(&frame, 5, 5), [255, 238, 221, 255], "the parent's");
    assert_eq!(
        pixel(&frame, 200, 110),
        [0, 128, 0, 255],
        "the frame's, in its hole"
    );
    assert_eq!(
        pixel(&frame, 160, 70),
        [0, 0, 255, 255],
        "the parent's over the frame"
    );

    // As one engine draws the same page.
    let mut engine = Engine::with_fonts(fonts());
    engine.set_viewport(SIZE.0, SIZE.1);
    engine.navigate(url(&server, "a.localhost", "/plain"));
    let began = Instant::now();
    while !(*engine.load_state() == LoadState::Loaded && engine.frames_settled()) {
        engine.poll_network();
        engine.run_tasks();
        assert!(began.elapsed() < WAIT);
        thread::sleep(Duration::from_millis(2));
    }
    let own = engine.frame_at(SIZE.0, SIZE.1, FrameTime::at(Duration::ZERO));
    let differ = frame
        .as_chunks::<4>()
        .0
        .iter()
        .zip(own.as_chunks::<4>().0)
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(differ, 0, "pixels that differ from one engine's");
}

#[test]
fn a_frames_load_messages_and_clicks_cross_the_renderers() {
    let server = server();
    let mut tab = tab();
    tab.frame_at(SIZE.0, SIZE.1, FrameTime::at(Duration::ZERO));
    tab.navigate(url(&server, "a.localhost", "/parent"));
    let mut lines = Vec::new();
    let b = format!("http://b.localhost:{}", server.port());
    let a = format!("http://a.localhost:{}", server.port());
    settle(&mut tab, &mut lines, "load and messages", |_, lines| {
        lines.iter().any(|line| line == "framed load")
            && lines.iter().any(|line| {
                line.ends_with("child got back ".to_owned().as_str())
                    || line.contains("child got back")
            })
    });
    assert!(
        lines.contains(&format!("parent got hello {b}")),
        "the browser's word on the sender's origin: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains(&format!("child got back {a}"))),
        "{lines:?}"
    );
    composed(&mut tab);
    // A click in the frame is the frame's, in its own coordinates.
    tab.pointer_move(50.0, 40.0);
    tab.pointer_down(50.0, 40.0, Button::Primary, 1);
    tab.pointer_up(50.0, 40.0, Button::Primary, 1);
    settle(&mut tab, &mut lines, "the click", |_, lines| {
        lines.iter().any(|line| line.contains("child click 30 20"))
    });
    // One on the parent's content over the frame is the parent's.
    tab.pointer_move(160.0, 70.0);
    tab.pointer_down(160.0, 70.0, Button::Primary, 1);
    tab.pointer_up(160.0, 70.0, Button::Primary, 1);
    settle(&mut tab, &mut lines, "the overlay's click", |_, lines| {
        lines.iter().any(|line| line == "overlay click")
    });
    assert!(
        !lines.iter().any(|line| line.contains("child click 140 50")),
        "{lines:?}"
    );
}

#[test]
fn a_sandboxed_frame_of_another_site_runs_no_scripts() {
    let server = server();
    let mut tab = tab();
    tab.navigate(url(&server, "a.localhost", "/sandboxed"));
    let mut lines = Vec::new();
    settle(&mut tab, &mut lines, "the frame shown", |tab, _| {
        tab.frame_roots().iter().any(|(_, url)| {
            url.as_ref()
                .is_some_and(|url| url.as_str().ends_with("/child"))
        })
    });
    thread::sleep(Duration::from_millis(300));
    tab.poll_network();
    lines.extend(tab.take_console().into_iter().map(|entry| entry.text));
    assert!(
        !lines.iter().any(|line| line.contains("child ran")),
        "{lines:?}"
    );
}

#[test]
fn a_frames_view_goes_with_its_element() {
    let server = server();
    let mut tab = tab();
    tab.navigate(url(&server, "a.localhost", "/removed"));
    let mut lines = Vec::new();
    settle(&mut tab, &mut lines, "the frame shown", |tab, _| {
        !tab.frame_roots().is_empty()
    });
    settle(&mut tab, &mut lines, "the frame gone", |tab, lines| {
        lines.iter().any(|line| line == "removed") && tab.frame_roots().is_empty()
    });
}

#[test]
fn keys_go_to_the_frame_last_clicked_in() {
    let server = TestServer::plain(|request| {
        let host = request.header("Host").unwrap_or_default().to_owned();
        let port = host.rsplit(':').next().unwrap_or_default().to_owned();
        let body = match request.target.as_str() {
            "/top" => format!(
                "<body style='margin: 0'><iframe src='http://b.localhost:{port}/field' \
                 style='border: 0; width: 300px; height: 100px'></iframe>"
            ),
            _ => "<body style='margin: 0'><input id=i style='width: 200px; height: 40px'>\
                  <script>document.getElementById('i').oninput = e => \
                  console.log('typed', e.target.value)</script>"
                .to_owned(),
        };
        Reply::ok("text/html", body.as_bytes())
    });
    let mut tab = tab();
    tab.frame_at(SIZE.0, SIZE.1, FrameTime::at(Duration::ZERO));
    tab.navigate(url(&server, "a.localhost", "/top"));
    let mut lines = Vec::new();
    settle(&mut tab, &mut lines, "the frame shown", |tab, _| {
        *tab.load_state() == LoadState::Loaded && !tab.frame_roots().is_empty()
    });
    composed(&mut tab);
    tab.pointer_move(20.0, 20.0);
    tab.pointer_down(20.0, 20.0, Button::Primary, 1);
    tab.pointer_up(20.0, 20.0, Button::Primary, 1);
    settle(&mut tab, &mut lines, "the field focused", |tab, _| {
        tab.can_paste()
    });
    tab.insert_text("hi");
    settle(&mut tab, &mut lines, "the text typed", |_, lines| {
        lines.iter().any(|line| line.ends_with("typed hi"))
    });
}

#[test]
fn a_frame_shown_elsewhere_stays_so_when_its_parent_navigates_it_home() {
    let server = server();
    let mut tab = tab();
    tab.navigate(url(&server, "a.localhost", "/back"));
    let mut lines = Vec::new();
    settle(
        &mut tab,
        &mut lines,
        "the frame back on the parent's site",
        |tab, _| {
            tab.frame_roots().iter().any(|(_, url)| {
                url.as_ref()
                    .is_some_and(|url| url.as_str().ends_with("/again"))
            })
        },
    );
    let roots = tab.frame_roots();
    assert_eq!(roots.len(), 1, "{roots:?}");
}

#[test]
fn a_parents_load_waits_for_its_frame_handed_over_however_late_the_browser_says_so() {
    let server = server();
    let mut tab = tab();
    tab.navigate(url(&server, "a.localhost", "/counted"));
    // The renderers go on without the tab. The frame shows a document of
    // its parent's site, then is sent to another site while its parent
    // still loads (an image comes late): the parent's renderer hears the
    // navigation moved long before the browser tells it the frame is
    // another renderer's, and its load must wait for that frame's.
    thread::sleep(Duration::from_millis(2000));
    let mut lines = Vec::new();
    settle(&mut tab, &mut lines, "the parent's load", |_, lines| {
        lines.iter().any(|line| line.starts_with("load "))
    });
    assert!(lines.contains(&"load 1".to_owned()), "{lines:?}");
}
