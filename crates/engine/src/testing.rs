//! What the engine's tests share: an engine over the bundled font with
//! its waker, a scripted server on 127.0.0.1, and ways to wait for both
//! and to look at what was shown.

use std::sync::atomic::{AtomicU64, Ordering};
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

/// Navigates `view`, an engine or a tab of renderers, to `url` and takes
/// what comes until it shows `url` loaded.
pub(crate) fn load_view(view: &mut dyn crate::view::View, url: Url) {
    view.navigate(url.clone());
    let deadline = Instant::now() + WAIT;
    while !(view.url() == Some(&url) && *view.load_state() == crate::LoadState::Loaded) {
        view.poll_network();
        view.run_tasks();
        assert!(
            Instant::now() < deadline,
            "{url} did not load: {:?}",
            view.load_state()
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
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

    /// Runs the engine as the shell does, taking results and running
    /// the tasks due as the waker says they come (M31: a worker's
    /// messages), until `done` holds of the console's lines so far,
    /// which it gives.
    pub(crate) fn run_until(&mut self, mut done: impl FnMut(&[String]) -> bool) -> Vec<String> {
        let deadline = Instant::now() + WAIT;
        let mut lines = Vec::new();
        loop {
            self.engine.poll_network();
            self.engine.run_tasks();
            lines.extend(
                self.engine
                    .take_console()
                    .into_iter()
                    .map(|entry| entry.text),
            );
            if done(&lines) {
                return lines;
            }
            let left = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_else(|| panic!("the engine gets there in time: {lines:#?}"));
            // Until the next task is due, as the shell waits, or a wake.
            let wait = self
                .engine
                .next_task_at()
                .map_or(left, |due| due.saturating_sub(self.engine.clock_now()));
            let _ = self.woken.recv_timeout(left.min(wait));
        }
    }

    /// Gives the engine a clock the test moves by hand (M26), at 0.
    pub(crate) fn hand_clock(&mut self) -> HandClock {
        let clock = HandClock::default();
        self.engine.set_clock(clock.clock());
        clock
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

/// A clock a test moves by hand (M26): the engine's tasks, timers and
/// scripts read it, so that no test waits for time to pass.
#[derive(Clone, Default)]
pub(crate) struct HandClock(Arc<AtomicU64>);

impl HandClock {
    /// Moves the clock on by `ms` milliseconds.
    pub(crate) fn advance(&self, ms: u64) {
        self.0.fetch_add(ms * 1_000_000, Ordering::SeqCst);
    }

    /// The clock as the engine reads it.
    pub(crate) fn clock(&self) -> crate::Clock {
        let nanos = self.0.clone();
        Arc::new(move || Duration::from_nanos(nanos.load(Ordering::SeqCst)))
    }
}

/// The load of the navigation under way, if there is one.
pub(crate) fn incoming(engine: &Engine) -> Option<&Incoming> {
    engine
        .navigation
        .as_ref()
        .map(|navigation| &navigation.incoming)
}

/// The class each style rule in force on an 800 by 600 screen begins
/// with, in cascade order: the rules of the sheets that apply, those of
/// the sheets they import among them, inside `@media` blocks that hold
/// there, `@supports` blocks that hold and `@layer` blocks.
pub(crate) fn names(engine: &Engine) -> Vec<String> {
    let media = style::Media::screen(800.0, 600.0);
    let mut names = Vec::new();
    for sheet in engine.stylesheets() {
        names_in(&sheet.rules, &media, &mut names);
    }
    names
}

fn names_in(rules: &[css::Rule], media: &style::Media, names: &mut Vec<String>) {
    for rule in rules {
        match rule {
            css::Rule::Qualified(rule) => {
                if let [css::Token::Delim('.'), css::Token::Ident(name), ..] =
                    rule.prelude.as_slice()
                {
                    names.push(name.clone());
                }
            }
            css::Rule::At(at) => {
                let Some(css::Block::Rules(inner)) = &at.block else {
                    continue;
                };
                let holds = match at.name.to_ascii_lowercase().as_str() {
                    "media" => style::parse_media_query_list(&at.prelude).evaluate(media),
                    "supports" => style::supports_condition(&at.prelude) == Some(true),
                    "layer" => true,
                    _ => false,
                };
                if holds {
                    names_in(inner, media, names);
                }
            }
        }
    }
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

/// An HTML page, never kept by the HTTP cache: since M38 a document is,
/// and a traversal would take it from there, where the tests mean a
/// document fetched again. A test of the cache serves its own.
pub(crate) fn html(body: &str) -> Reply {
    Reply::with_fields(
        200,
        &[("Content-Type", "text/html"), ("Cache-Control", "no-store")],
        body.as_bytes(),
    )
}

pub(crate) fn css(body: &str) -> Reply {
    Reply::ok("text/css", body.as_bytes())
}

pub(crate) fn css_bytes(body: &[u8]) -> Reply {
    Reply::ok("text/css", body)
}

/// The paths the server was asked for, in the order it was, but for the
/// icons the pages asked for (M15), which tests of documents and their
/// stylesheets leave aside.
pub(crate) fn paths(server: &TestServer) -> Vec<String> {
    server
        .requests()
        .into_iter()
        .filter(|request| {
            request
                .header("Accept")
                .is_none_or(|accept| !accept.starts_with("image/"))
        })
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

/// A sink that keeps what it is given and plays it at once (M40).
/// Its samples may be taken; what it played only grows. Paced, it plays
/// at the clock, as a device does; else at once.
#[derive(Default)]
pub(crate) struct Capture {
    pub(crate) samples: Mutex<Vec<f32>>,
    played: AtomicU64,
    paced: Option<crate::audio_output::ClockSink>,
}

impl Capture {
    pub(crate) fn paced() -> Capture {
        Capture {
            paced: Some(crate::audio_output::ClockSink::new()),
            ..Capture::default()
        }
    }
}

impl crate::AudioSink for Capture {
    fn write(&self, samples: &[f32]) {
        self.samples
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(samples);
        self.played.fetch_add(
            (samples.len() / crate::audio_output::OUTPUT_CHANNELS) as u64,
            Ordering::Relaxed,
        );
        if let Some(clock) = &self.paced {
            clock.write(samples);
        }
    }

    fn played(&self) -> u64 {
        match &self.paced {
            Some(clock) => clock.played(),
            None => self.played.load(Ordering::Relaxed),
        }
    }
}

/// The process's mixer is one: the tests that run an `AudioContext` take
/// turns (M40).
pub(crate) fn audio_turn() -> std::sync::MutexGuard<'static, ()> {
    static TURN: Mutex<()> = Mutex::new(());
    TURN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A fifth of a second of `samples`' sound (interleaved stereo at 48 kHz)
/// with no gap in it, from where its left channel is louder than `loud`
/// (a busy machine's mixer may run dry a moment): where its left channel
/// never stays at silence a millisecond (M40).
pub(crate) fn unbroken(samples: &[f32], loud: f32) -> &[f32] {
    let frames = samples.len() / 2;
    let window = 9600;
    let mut at = (0..frames)
        .find(|&i| samples[i * 2].abs() > loud)
        .expect("sound");
    while at + window <= frames {
        let left: Vec<f32> = samples[at * 2..(at + window) * 2]
            .iter()
            .step_by(2)
            .copied()
            .collect();
        let gap = left.windows(48).position(|w| w.iter().all(|s| *s == 0.0));
        match gap {
            None => return &samples[at * 2..(at + window) * 2],
            Some(g) => at += g + 48,
        }
    }
    panic!("no fifth of a second without a gap");
}
