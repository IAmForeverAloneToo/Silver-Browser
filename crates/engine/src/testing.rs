//! What the engine's tests share: an engine over the bundled font with
//! its waker, a scripted server on 127.0.0.1, and ways to wait for both
//! and to look at what was shown.

use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use dom::NodeId;
use net::test_server::{Reply, TestServer};

use crate::loader::Incoming;
use crate::{Engine, Url};

/// Long enough for anything a test waits for on a loaded machine.
pub(crate) const WAIT: Duration = Duration::from_secs(30);

/// An engine over the bundled font, and the channel its waker sends on.
pub(crate) struct Tab {
    pub(crate) engine: Engine,
    woken: Receiver<()>,
}

impl Tab {
    pub(crate) fn new() -> Tab {
        let mut engine = Engine::with_fonts(
            text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
                .expect("the bundled font parses"),
        );
        let (wake, woken) = channel();
        engine.set_waker(move || {
            let _ = wake.send(());
        });
        Tab { engine, woken }
    }

    /// Takes results as the waker says they come, as the shell does,
    /// until `done` holds.
    pub(crate) fn wait_until(&mut self, mut done: impl FnMut(&Engine) -> bool) {
        let deadline = Instant::now() + WAIT;
        loop {
            self.engine.poll_network();
            if done(&self.engine) {
                return;
            }
            let left = deadline
                .checked_duration_since(Instant::now())
                .expect("the engine gets there in time");
            self.woken
                .recv_timeout(left)
                .expect("a fetch finishes in time");
        }
    }

    /// Takes results until no fetch is in flight.
    pub(crate) fn settle(&mut self) {
        self.wait_until(|engine| engine.network.in_flight() == 0);
    }

    /// Loads `url` and takes results until no fetch is in flight.
    pub(crate) fn load(&mut self, url: Url) {
        self.engine.navigate(url);
        self.settle();
    }

    /// The name each stylesheet that applies goes by: its first rule's
    /// first identifier, `a` for `.a {}`.
    pub(crate) fn names(&self) -> Vec<String> {
        names(&self.engine)
    }
}

/// The load of the navigation under way, if there is one.
pub(crate) fn incoming(engine: &Engine) -> Option<&Incoming> {
    engine
        .navigation
        .as_ref()
        .map(|navigation| &navigation.incoming)
}

pub(crate) fn names(engine: &Engine) -> Vec<String> {
    engine
        .stylesheets()
        .iter()
        .map(|sheet| {
            sheet
                .rules
                .iter()
                .find_map(|rule| match rule {
                    css::Rule::Qualified(rule) => {
                        rule.prelude.iter().find_map(|token| match token {
                            css::Token::Ident(name) => Some(name.clone()),
                            _ => None,
                        })
                    }
                    css::Rule::At(_) => None,
                })
                .unwrap_or_default()
        })
        .collect()
}

/// A server answering each path with what `route` gives, and 404 for
/// the paths it gives nothing for.
pub(crate) fn serve(route: impl Fn(&str) -> Option<Reply> + Send + Sync + 'static) -> TestServer {
    TestServer::plain(move |request| {
        route(&request.target).unwrap_or_else(|| {
            Reply::with_fields(404, &[("Content-Type", "text/plain")], b"not here")
        })
    })
}

pub(crate) fn html(body: &str) -> Reply {
    Reply::ok("text/html", body.as_bytes())
}

pub(crate) fn css(body: &str) -> Reply {
    Reply::ok("text/css", body.as_bytes())
}

pub(crate) fn css_bytes(body: &[u8]) -> Reply {
    Reply::ok("text/css", body)
}

/// The paths the server was asked for, in the order it was.
pub(crate) fn paths(server: &TestServer) -> Vec<String> {
    server
        .requests()
        .into_iter()
        .map(|request| request.target)
        .collect()
}

/// The paths the server was asked for, sorted: fetches made together
/// reach it in any order.
pub(crate) fn sorted_paths(server: &TestServer) -> Vec<String> {
    let mut paths = paths(server);
    paths.sort_unstable();
    paths
}

/// Waits for `done`, which no fetch result tells of.
pub(crate) fn wait_for(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "it happens in time");
        thread::sleep(Duration::from_millis(2));
    }
}

/// A gate a server's script waits at until the test opens it.
#[derive(Default)]
pub(crate) struct Gate {
    open: Mutex<bool>,
    opened: Condvar,
}

impl Gate {
    pub(crate) fn new() -> Arc<Gate> {
        Arc::new(Gate::default())
    }

    pub(crate) fn wait(&self) {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        while !*open {
            open = self
                .opened
                .wait(open)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    pub(crate) fn open(&self) {
        *self.open.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.opened.notify_all();
    }
}

pub(crate) fn element(engine: &Engine, tag: &str) -> NodeId {
    let document = engine.document();
    document
        .subtree(document.root())
        .find(|&node| document.tag(node) == Some(tag))
        .expect("the element")
}

pub(crate) fn by_id(engine: &Engine, id: &str) -> NodeId {
    let document = engine.document();
    document
        .subtree(document.root())
        .find(|&node| document.attr(node, "id") == Some(id))
        .expect("the element")
}

pub(crate) fn color(engine: &mut Engine, id: &str) -> (f32, f32, f32) {
    let node = by_id(engine, id);
    let color = engine.styles().get(node).expect("styled").color;
    (color.r, color.g, color.b)
}

pub(crate) fn text(engine: &Engine) -> String {
    engine.document().text_content(engine.document().root())
}

pub(crate) const RED: (f32, f32, f32) = (1.0, 0.0, 0.0);
pub(crate) const BLACK: (f32, f32, f32) = (0.0, 0.0, 0.0);
