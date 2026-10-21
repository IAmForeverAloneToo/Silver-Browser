//! A compromised renderer (M34.7, docs/design/processes.md, "The
//! compromised renderer"): a renderer that speaks the protocol itself,
//! locked to its site as any other, asks the browser for what it may
//! not. Each is refused: the renderer is killed where the protocol says
//! so, its page says it stopped, and the tab goes on, its next page
//! shown by an honest renderer.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use net::cookies::Context;
use net::test_server::{Reply, TestServer};
use net::{Destination, Mode, NetworkError, Origin, Partition, Request, Url};

use super::{Launched, Launcher, RemoteTab, RendererProcess, ThreadLauncher};
use crate::ipc::{self, Bytes, StorageOp, ToBrowser, ToRenderer, WireTask};
use crate::view::View;
use crate::{FrameTime, LoadError, LoadState};

const WAIT: Duration = Duration::from_secs(20);

fn fonts() -> text::FontCollection {
    text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
}

/// What a compromised renderer writes, once it has its view.
enum Said {
    /// Bytes as they are, framed or not.
    Raw(Vec<u8>),
    /// Its end of the channel closed.
    End,
}

/// `message`, as the protocol frames it.
fn said(message: ToBrowser) -> Said {
    Said::Raw(ipc::frame(&message).expect("framed"))
}

/// What a compromised renderer says, given its view's frame number.
type Attack = Box<dyn FnOnce(u32) -> Vec<Said> + Send>;

/// A launcher whose next renderers each make one attack of `attacks`;
/// past them, renderers are honest engines on threads. What the browser
/// tells the compromised ones goes to `heard`.
struct Compromised {
    attacks: Mutex<VecDeque<Attack>>,
    honest: ThreadLauncher,
    heard: Mutex<mpsc::Sender<ToRenderer>>,
}

impl Launcher for Compromised {
    fn launch(&self, number: u32) -> std::io::Result<Launched> {
        let attack = self
            .attacks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front();
        let Some(attack) = attack else {
            return self.honest.launch(number);
        };
        let ((browser_in, browser_out), (mut renderer_in, renderer_out)) = ipc::duplex();
        let breakers = [browser_out.breaker(), renderer_out.breaker()];
        let heard = self
            .heard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        thread::spawn(move || {
            let mut out = Some(renderer_out);
            let write = |out: &mut Option<ipc::PipeWriter>, bytes: &[u8]| {
                if let Some(out) = out {
                    let _ = out.write_all(bytes);
                }
            };
            let Ok(Some(ToRenderer::Hello { .. })) =
                ipc::read_message::<ToRenderer>(&mut renderer_in)
            else {
                return;
            };
            write(
                &mut out,
                &ipc::frame(&ToBrowser::Hello {
                    version: ipc::IPC_VERSION,
                })
                .expect("a hello"),
            );
            let frame = loop {
                match ipc::read_message::<ToRenderer>(&mut renderer_in) {
                    Ok(Some(ToRenderer::CreateView { frame, .. })) => break frame,
                    Ok(Some(_)) => {}
                    _ => return,
                }
            };
            for said in attack(frame) {
                match said {
                    Said::Raw(bytes) => write(&mut out, &bytes),
                    Said::End => out = None,
                }
            }
            while let Ok(Some(message)) = ipc::read_message::<ToRenderer>(&mut renderer_in) {
                let _ = heard.send(message);
            }
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

/// A server of every site's pages: `/page` a plain page, `/secret` HTML
/// another site's renderer must not read.
fn server() -> TestServer {
    TestServer::plain(|request| match request.target.as_str() {
        "/secret" => Reply::ok("text/html", b"<p>the victim's secret</p>"),
        _ => Reply::ok("text/html", b"<p>a page</p>"),
    })
}

fn url(server: &TestServer, host: &str, path: &str) -> Url {
    Url::parse(&format!("http://{host}:{}{path}", server.port())).expect("a URL")
}

fn victim(server: &TestServer) -> Url {
    url(server, "victim.localhost", "/secret")
}

/// A fetch of `url` for the renderer's frame.
fn fetch(frame: u32, request: Request) -> Said {
    said(ToBrowser::Fetch {
        id: 1,
        frame,
        streamed: false,
        request: Box::new(request),
    })
}

/// The attacks the browser must answer with a kill, and a word of why.
fn killing(server: &TestServer) -> Vec<(&'static str, Attack, &'static str)> {
    let other = victim(server);
    let origin = other.origin();
    let (o1, o2, o3, o4, o5, o6) = (
        origin.clone(),
        origin.clone(),
        origin.clone(),
        origin.clone(),
        origin.clone(),
        origin.clone(),
    );
    let (u1, u2, u3) = (other.clone(), other.clone(), other.clone());
    let named = origin.serialization();
    let navigation = other.clone();
    let (u4, u5, u6) = (other.clone(), other.clone(), other.clone());
    let (u7, u8) = (other.clone(), other.clone());
    vec![
        (
            "a forged cookie to another site",
            Box::new(move |frame| {
                let mut request = Request::get(u7, Destination::Image);
                request.origin = Some(Origin::new_opaque());
                request.headers.append(b"Cookie", b"session=forged");
                vec![fetch(frame, request)]
            }) as Attack,
            "the field Cookie",
        ),
        (
            "fetch metadata forged",
            Box::new(move |frame| {
                let mut request = Request::get(u8, Destination::Image);
                request.origin = Some(Origin::new_opaque());
                request.headers.append(b"Sec-Fetch-Site", b"same-origin");
                vec![fetch(frame, request)]
            }),
            "the field Sec-Fetch-Site",
        ),
        (
            "a forbidden method",
            Box::new(move |frame| {
                let mut request = Request::get(
                    Url::parse("http://a.test/").expect("a URL"),
                    Destination::Empty,
                );
                request.origin = Some(Origin::new_opaque());
                request.method = net::Method::parse("TRACE").expect("a method");
                vec![fetch(frame, request)]
            }),
            "the method TRACE",
        ),
        (
            "another site's HTML by a request of no origin",
            Box::new(move |frame| {
                let mut request = Request::get(u4, Destination::Script);
                request.mode = Mode::NoCors;
                vec![fetch(frame, request)]
            }),
            "a request of no origin",
        ),
        (
            "another site's HTML in a navigation's mode",
            Box::new(move |frame| {
                let mut request = Request::get(u5, Destination::Script);
                request.mode = Mode::Navigate;
                vec![fetch(frame, request)]
            }),
            "a fetch in a navigation's mode",
        ),
        (
            "another site's document, not streamed to be handed over",
            Box::new(move |frame| vec![fetch(frame, Request::get(u6, Destination::Document))]),
            "not a navigation's",
        ),
        (
            "another site's cookies by document.cookie",
            Box::new(move |frame| {
                vec![said(ToBrowser::Cookies {
                    call: 1,
                    frame,
                    url: u1,
                    context: Context::SAME_SITE,
                })]
            }),
            "the cookies of",
        ),
        (
            "another site's cookies written",
            Box::new(move |frame| {
                vec![said(ToBrowser::SetCookie {
                    frame,
                    url: u2,
                    value: Bytes(b"stolen=1".to_vec()),
                    context: Context::SAME_SITE,
                })]
            }),
            "the cookies of",
        ),
        (
            "a request claiming another site's origin",
            Box::new(move |frame| {
                let mut request = Request::get(u3, Destination::Empty);
                request.origin = Some(o1);
                vec![fetch(frame, request)]
            }),
            "a request of",
        ),
        (
            "a request in another site's partition",
            Box::new(move |frame| {
                let mut request = Request::get(
                    Url::parse("http://a.test/").expect("a URL"),
                    Destination::Empty,
                );
                request.origin = Some(Origin::new_opaque());
                request.partition = Some(Partition {
                    top: o2,
                    cross_site: false,
                });
                vec![fetch(frame, request)]
            }),
            "a request in the partition of",
        ),
        (
            "another site's storage",
            Box::new(move |frame| {
                vec![said(ToBrowser::Storage {
                    call: 1,
                    frame,
                    local: true,
                    origin: o3,
                    partition: None,
                    op: StorageOp::Get {
                        item: "secret".into(),
                    },
                })]
            }),
            "the storage of",
        ),
        (
            "another site's storage changed",
            Box::new(move |frame| {
                vec![said(ToBrowser::StorageChanged {
                    frame,
                    local: true,
                    origin: o6,
                    partition: None,
                    key: Some("secret".into()),
                    old: None,
                    new: Some("forged".into()),
                    url: String::new(),
                })]
            }),
            "the storage of",
        ),
        (
            "a frame it does not host, posted from",
            Box::new(move |frame| {
                vec![said(ToBrowser::PostFrame {
                    from: Some(frame + 1000),
                    to: frame,
                    task: WireTask::Message {
                        data: String::new(),
                        origin: "null".into(),
                        target_origin: "*".into(),
                    },
                })]
            }),
            "is not renderer",
        ),
        (
            "a frame it does not host, dropped",
            Box::new(move |frame| vec![said(ToBrowser::FrameDropped { key: frame + 1000 })]),
            "not of the renderer's pages",
        ),
        (
            "a new frame under one it does not host",
            Box::new(move |frame| {
                vec![said(ToBrowser::NewFrame {
                    call: 1,
                    parent: frame + 1000,
                })]
            }),
            "is not renderer",
        ),
        (
            "a message stamped with another origin",
            Box::new(move |frame| {
                vec![said(ToBrowser::PostFrame {
                    from: Some(frame),
                    to: frame,
                    task: WireTask::Message {
                        data: String::new(),
                        origin: named,
                        target_origin: "*".into(),
                    },
                })]
            }),
            "a message from",
        ),
        (
            "a navigation in another site's name",
            Box::new(move |frame| {
                vec![said(ToBrowser::PostFrame {
                    from: Some(frame),
                    to: frame,
                    task: WireTask::Navigate {
                        url: Box::new(navigation),
                        replace: false,
                        post: None,
                        origin: o4,
                    },
                })]
            }),
            "a navigation by",
        ),
        (
            "a page's renderer speaking as the DevTools panel",
            Box::new(move |_| {
                vec![said(ToBrowser::DevTools {
                    view: 1,
                    message: r#"{"id":1,"method":"browser.version"}"#.to_owned(),
                })]
            }),
            "DevTools",
        ),
        (
            "a surface it was not given",
            Box::new(move |_| {
                vec![said(ToBrowser::Frame {
                    view: 999,
                    frame: 1,
                    width: 1,
                    height: 1,
                })]
            }),
            "view 999",
        ),
        (
            "a local network request in another site's name",
            Box::new(move |frame| {
                let mut request = Request::get(
                    Url::parse("http://a.test/").expect("a URL"),
                    Destination::Empty,
                );
                request.origin = Some(Origin::new_opaque());
                request.local_network = Some(net::LocalNetworkAccess {
                    origin: o5,
                    local: true,
                    loopback: true,
                });
                vec![fetch(frame, request)]
            }),
            "local network access for",
        ),
        (
            "a message past the size limit",
            Box::new(|_| {
                let mut bytes = ((ipc::MAX_MESSAGE + 1) as u32).to_le_bytes().to_vec();
                bytes.extend_from_slice(&[0; 64]);
                vec![Said::Raw(bytes)]
            }),
            "a message of",
        ),
        (
            "an unknown tag",
            Box::new(|_| {
                let mut bytes = 2u32.to_le_bytes().to_vec();
                bytes.extend_from_slice(&250u16.to_le_bytes());
                vec![Said::Raw(bytes)]
            }),
            "unknown tag 250",
        ),
        (
            "a truncated message",
            Box::new(|_| {
                let mut bytes = 100u32.to_le_bytes().to_vec();
                bytes.extend_from_slice(&[1; 10]);
                vec![Said::Raw(bytes), Said::End]
            }),
            "stopped",
        ),
    ]
}

/// A tab of renderers whose next ones make `attacks`, and what the
/// compromised ones hear.
fn tab(attacks: Vec<Attack>) -> (RemoteTab, mpsc::Receiver<ToRenderer>) {
    let (heard, hearing) = mpsc::channel();
    let launcher = Arc::new(Compromised {
        attacks: Mutex::new(attacks.into()),
        honest: ThreadLauncher::with_fonts(fonts()),
        heard: Mutex::new(heard),
    });
    let mut tab = RemoteTab::with_cap(launcher, fonts(), 16, false);
    tab.set_viewport(200, 100);
    (tab, hearing)
}

/// Takes what the renderers say until `done` holds of the tab.
fn settle(tab: &mut RemoteTab, what: &str, mut done: impl FnMut(&mut RemoteTab) -> bool) {
    let began = Instant::now();
    while !done(tab) {
        tab.poll_network();
        tab.run_tasks();
        assert!(began.elapsed() < WAIT, "{what}: {:?}", tab.load_state());
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn what_a_compromised_renderer_asks_for_kills_it_and_the_tab_goes_on() {
    let server = server();
    let attacks = killing(&server);
    let expected: Vec<(&str, &str)> = attacks.iter().map(|(name, _, why)| (*name, *why)).collect();
    let (mut tab, _) = tab(attacks.into_iter().map(|(_, attack, _)| attack).collect());
    for (n, (name, why)) in expected.iter().enumerate() {
        // A site of its own for each attack, so that each starts a
        // renderer of its own.
        let host = format!("attacker{n}.localhost");
        tab.navigate(url(&server, &host, "/page"));
        let mut notice = String::new();
        settle(&mut tab, name, |tab| {
            if let Some(said) = tab.take_notice() {
                notice = said;
            }
            !notice.is_empty() && *tab.load_state() == LoadState::Failed(LoadError::Lost)
        });
        assert!(notice.contains(why), "{name}: {notice}");
        assert!(
            tab.renderers().is_empty(),
            "{name}: the renderer lives: {:?}",
            tab.renderers()
        );
    }
    // The browser goes on: an honest renderer shows the next page.
    let page = url(&server, "honest.localhost", "/page");
    tab.navigate(page.clone());
    settle(&mut tab, "the honest page", |tab| {
        tab.url() == Some(&page) && *tab.load_state() == LoadState::Loaded
    });
    let frame = tab.frame_at(200, 100, FrameTime::at(Duration::ZERO));
    assert_eq!(frame.len(), 200 * 100 * 4);
}

/// What a compromised renderer that asks for `request` hears of it: the
/// fetch's end, the body's pieces that came before it in its response.
fn fetched(
    server: &TestServer,
    streamed: bool,
    request: impl FnOnce(u32) -> Request + Send + 'static,
) -> ToRenderer {
    let attack: Attack = Box::new(move |frame| {
        vec![said(ToBrowser::Fetch {
            id: 1,
            frame,
            streamed,
            request: Box::new(request(frame)),
        })]
    });
    let (mut tab, heard) = tab(vec![attack]);
    tab.navigate(url(server, "attacker.localhost", "/page"));
    let began = Instant::now();
    let mut body = Vec::new();
    loop {
        tab.poll_network();
        match heard.try_recv() {
            Ok(ToRenderer::FetchBody { id: 1, bytes }) => body.extend(bytes.0),
            Ok(ToRenderer::FetchDone {
                id: 1,
                mut result,
                violations,
            }) => {
                assert!(
                    tab.take_notice().is_none(),
                    "not a kill: the browser answers"
                );
                if let Ok(response) = &mut result {
                    body.append(&mut response.body);
                    response.body = body;
                }
                return ToRenderer::FetchDone {
                    id: 1,
                    result,
                    violations,
                };
            }
            _ => {}
        }
        assert!(began.elapsed() < WAIT, "no answer");
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn a_compromised_renderer_reads_no_other_sites_html_by_no_cors() {
    let server = server();
    let secret = victim(&server);
    let page = url(&server, "attacker.localhost", "/page").origin();
    let done = fetched(&server, false, move |_| {
        let mut request = Request::get(secret, Destination::Script);
        request.mode = Mode::NoCors;
        request.origin = Some(page);
        request
    });
    let ToRenderer::FetchDone {
        result: Ok(response),
        ..
    } = done
    else {
        panic!("{done:?}");
    };
    assert!(response.body.is_empty(), "{:?}", response.body);
}

#[test]
fn a_compromised_renderer_reads_no_file() {
    let server = server();
    let file = std::env::temp_dir().join(format!("silver-compromised-{}", std::process::id()));
    std::fs::write(&file, "the user's").expect("written");
    let target = Url::from_file_path(&file).expect("a URL");
    let page = url(&server, "attacker.localhost", "/page").origin();
    let done = fetched(&server, false, move |_| {
        let mut request = Request::get(target, Destination::Empty);
        request.origin = Some(page);
        request
    });
    let _ = std::fs::remove_file(&file);
    assert!(
        matches!(
            done,
            ToRenderer::FetchDone {
                result: Err(NetworkError::Refused(_)),
                ..
            }
        ),
        "{done:?}"
    );
}

#[test]
fn a_compromised_renderer_has_no_body_of_another_sites_download() {
    let server = TestServer::plain(|request| match request.target.as_str() {
        "/export" => Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/csv"),
                ("Content-Disposition", "attachment; filename=export.csv"),
            ],
            b"the victim's secret",
        ),
        _ => Reply::ok("text/html", b"<p>a page</p>"),
    });
    let export = url(&server, "victim.localhost", "/export");
    let done = fetched(&server, true, move |_| {
        Request::get(export, Destination::Document)
    });
    let ToRenderer::FetchDone {
        result: Ok(response),
        ..
    } = done
    else {
        panic!("{done:?}");
    };
    assert_eq!(
        response.status, 200,
        "the head, which says it is a download"
    );
    assert!(response.body.is_empty(), "{:?}", response.body);
}
