use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use super::process::start_within;
use super::*;
use crate::ipc::{self, IPC_VERSION, ToBrowser, ToRenderer};

const WAIT: Duration = Duration::from_secs(20);

fn nothing() -> crate::loader::Waker {
    Arc::new(|| {})
}

/// The events of renderer `number` until it is gone.
fn until_gone(events: &mpsc::Receiver<(u32, Event)>, number: u32) -> Vec<String> {
    let mut seen = Vec::new();
    loop {
        let (from, event) = events.recv_timeout(WAIT).expect("an event in time");
        assert_eq!(from, number);
        let gone = matches!(event, Event::Gone(_));
        seen.push(format!("{event:?}"));
        if gone {
            return seen;
        }
    }
}

#[test]
fn a_renderer_answers_the_hello_and_ends_when_told() {
    let (events, received) = mpsc::channel();
    let renderer =
        Renderer::start(&ThreadLauncher::default(), 3, events, nothing(), None).expect("started");
    assert!(renderer.wait_ready(WAIT));
    assert_eq!(renderer.number(), 3);
    assert_eq!(renderer.gone(), None);
    renderer.shutdown();
    assert_eq!(until_gone(&received, 3), ["Ready", "Gone(Shutdown)"]);
    assert_eq!(renderer.gone(), Some(Gone::Shutdown));
    let mut waited = Duration::ZERO;
    while renderer.exit_code().is_none() && waited < WAIT {
        thread::sleep(Duration::from_millis(5));
        waited += Duration::from_millis(5);
    }
    assert_eq!(renderer.exit_code(), Some(0), "its thread ended well");
}

#[test]
fn a_renderer_killed_is_gone() {
    let (events, received) = mpsc::channel();
    let renderer =
        Renderer::start(&ThreadLauncher::default(), 1, events, nothing(), None).expect("started");
    assert!(renderer.wait_ready(WAIT));
    renderer.bad_message("a test's");
    assert_eq!(
        until_gone(&received, 1),
        ["Ready", "Gone(BadMessage(\"a test's\"))"]
    );
    assert_eq!(renderer.gone(), Some(Gone::BadMessage("a test's".into())));
}

/// A launcher whose renderer is `body`, given the renderer's ends of the
/// pipes, and whether its process was killed.
struct Fake<F>(F, Arc<AtomicBool>);

/// A fake renderer's process: killing it breaks its pipes.
struct Breaking([ipc::PipeBreaker; 2], Arc<AtomicBool>);

impl RendererProcess for Breaking {
    fn kill(&mut self) {
        self.1.store(true, Ordering::Relaxed);
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

impl<F> Launcher for Fake<F>
where
    F: Fn(ipc::PipeReader, ipc::PipeWriter) + Send + Sync + Clone + 'static,
{
    fn launch(&self, _: u32) -> std::io::Result<Launched> {
        let ((browser_in, browser_out), (renderer_in, renderer_out)) = ipc::duplex();
        let breakers = [browser_out.breaker(), renderer_out.breaker()];
        let body = self.0.clone();
        thread::spawn(move || body(renderer_in, renderer_out));
        Ok(Launched {
            from_renderer: Box::new(browser_in),
            to_renderer: Box::new(browser_out),
            log: None,
            process: Box::new(Breaking(breakers, self.1.clone())),
        })
    }
}

fn hello(version: u32) -> Vec<u8> {
    ipc::frame(&ToBrowser::Hello { version }).expect("short")
}

/// What becomes of a renderer that writes `bytes` and then keeps its
/// pipes open until the browser is done with it; each way it goes, the
/// browser kills it.
fn written(bytes: Vec<u8>, timeout: Duration) -> Vec<String> {
    let (events, received) = mpsc::channel();
    let killed = Arc::new(AtomicBool::new(false));
    let fake = Fake(
        move |mut input: ipc::PipeReader, mut output: ipc::PipeWriter| {
            output.write_all(&bytes).expect("written");
            let mut sink = Vec::new();
            let _ = input.read_to_end(&mut sink);
        },
        killed.clone(),
    );
    let renderer = start_within(&fake, 9, events, nothing(), None, timeout).expect("started");
    // The fake's pipes stay open: the browser ends it by closing its own.
    let seen = loop {
        let (_, event) = received.recv_timeout(WAIT).expect("an event in time");
        if let Event::Gone(why) = event {
            break vec![format!("{why:?}")];
        }
    };
    assert!(killed.load(Ordering::Relaxed), "killed: {seen:?}");
    drop(renderer);
    seen
}

#[test]
fn a_renderer_that_does_not_answer_is_killed() {
    assert_eq!(written(Vec::new(), Duration::from_millis(50)), ["NoHello"]);
}

#[test]
fn a_hello_of_another_version_or_anything_else_first_is_a_bad_message() {
    assert_eq!(
        written(hello(IPC_VERSION + 1), WAIT),
        [format!(
            "BadMessage(\"Hello {{ version: {} }} before a hello of version {IPC_VERSION}\")",
            IPC_VERSION + 1
        )]
    );
    let mut twice = hello(IPC_VERSION);
    twice.extend(hello(IPC_VERSION));
    assert_eq!(written(twice, WAIT), ["BadMessage(\"a second hello\")"]);
    assert_eq!(
        written(vec![2, 0, 0, 0, 0x34, 0x12], WAIT),
        ["BadMessage(\"unknown tag 4660 of ToBrowser\")"]
    );
    let mut too_long = hello(IPC_VERSION);
    too_long.extend(((ipc::MAX_MESSAGE + 1) as u32).to_le_bytes());
    assert_eq!(
        written(too_long, WAIT),
        [format!(
            "BadMessage(\"a message of {} bytes\")",
            ipc::MAX_MESSAGE + 1
        )]
    );
}

#[test]
fn a_renderer_refuses_a_hello_of_another_version() {
    let ((browser_in, browser_out), (renderer_in, renderer_out)) = ipc::duplex();
    let to_renderer = ipc::Sender::<ToRenderer>::new(browser_out);
    to_renderer
        .send(&ToRenderer::Hello {
            version: IPC_VERSION + 1,
            process: 1,
        })
        .expect("sent");
    // On a thread, so that a renderer that took the hello fails the test
    // rather than wait for ever.
    let (done, ended) = mpsc::channel();
    thread::spawn(move || {
        let _ = done.send(crate::renderer::run(renderer_in, renderer_out));
    });
    let ended = ended.recv_timeout(WAIT).expect("the renderer ended");
    assert_eq!(
        ended,
        Err(format!(
            "a hello of version {}, not {IPC_VERSION}",
            IPC_VERSION + 1
        ))
    );
    drop(browser_in);
}

#[test]
fn each_renderer_numbers_its_opaque_origins_apart() {
    use crate::renderer::opaque_origins_of;
    assert!(opaque_origins_of(0) > 0, "apart from the browser's");
    assert!(opaque_origins_of(1) - opaque_origins_of(0) >= 1 << 40);
    let (events, _received) = mpsc::channel();
    let renderer =
        Renderer::start(&ThreadLauncher::default(), 5, events, nothing(), None).expect("started");
    assert!(renderer.wait_ready(WAIT));
    // Renderers as threads share the process's counter, which the hello
    // moved past the renderer's space's start.
    let net::Origin::Opaque(number) = net::Origin::new_opaque() else {
        panic!("an opaque origin");
    };
    assert!(number >= opaque_origins_of(5), "{number}");
}
