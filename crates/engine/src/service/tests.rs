use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use super::process::{MOST_UNTAKEN_MESSAGES, start_within};
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

/// How the browser takes a flooding renderer's messages in a test.
#[derive(Clone, Copy)]
enum Taking {
    /// Each as it comes.
    Promptly,
    /// One each `0` milliseconds: it takes, but is outrun.
    Slowly(u64),
    /// Nothing for two seconds, as a browser held in a modal loop, then
    /// each as it comes.
    AfterAStall,
}

/// A renderer that says hello, writes `frames`, and keeps its pipes open
/// until it is killed, its messages taken as `taking` says: the reasons
/// it is gone for, whether it was killed, and how many messages the
/// browser took.
fn flooded(frames: Vec<Vec<u8>>, taking: Taking) -> (Vec<String>, bool, usize) {
    let (events, received) = mpsc::channel();
    let killed = Arc::new(AtomicBool::new(false));
    let total = frames.len();
    let fake = Fake(
        move |mut input: ipc::PipeReader, mut output: ipc::PipeWriter| {
            if output.write_all(&hello(IPC_VERSION)).is_err() {
                return;
            }
            for frame in &frames {
                if output.write_all(frame).is_err() {
                    return;
                }
            }
            let mut sink = Vec::new();
            let _ = input.read_to_end(&mut sink);
        },
        killed.clone(),
    );
    let renderer = start_within(&fake, 9, events, nothing(), None, WAIT).expect("started");
    if let Taking::AfterAStall = taking {
        thread::sleep(Duration::from_secs(2));
    }
    let began = Instant::now();
    let mut gone = Vec::new();
    let mut took = 0;
    // When the renderer was first seen killed: its end follows at once,
    // whatever it had sent (CI, 2026-11-26: a slow taker reached it after
    // the ~4096 messages queued before it).
    let mut killed_at: Option<Instant> = None;
    while took < total && gone.is_empty() {
        assert!(
            began.elapsed() < WAIT,
            "waited {WAIT:?}: {took} of {total} taken"
        );
        let Ok((_, event)) = received.recv_timeout(WAIT) else {
            break;
        };
        if killed_at.is_none() && killed.load(Ordering::Relaxed) {
            killed_at = Some(Instant::now());
        }
        match event {
            // As the tab drops it, unread.
            Event::Message(_, held) if held.is_from_a_hostile_renderer() => continue,
            Event::Message(..) => took += 1,
            Event::Gone(why) => {
                if let Some(at) = killed_at {
                    let late = at.elapsed();
                    assert!(
                        late < Duration::from_secs(3),
                        "its end came {late:?} after the kill"
                    );
                }
                gone.push(format!("{why:?}"));
            }
            Event::Ready => {}
        }
        if let Taking::Slowly(ms) = taking {
            thread::sleep(Duration::from_millis(ms));
        }
    }
    let was_killed = killed.load(Ordering::Relaxed);
    drop(renderer);
    (gone, was_killed, took)
}

fn past_its_budget(gone: &[String]) -> bool {
    gone.iter()
        .any(|why| why.contains("past its budget of untaken messages"))
}

#[test]
fn a_renderer_outrunning_a_browser_that_takes_is_ended_as_a_bad_message_source() {
    // The lead's ruling, 2026-11-26: a renderer whose messages the browser
    // takes, but slower than they come, may hold at most
    // MOST_UNTAKEN_MESSAGES of them untaken.
    let frame = ipc::frame(&ToBrowser::FocusLeft { view: 0 }).expect("short");
    let (gone, killed, _) = flooded(vec![frame; MOST_UNTAKEN_MESSAGES * 3], Taking::Slowly(2));
    assert!(killed && past_its_budget(&gone), "{gone:?}");
}

#[test]
fn a_renderer_past_its_bytes_of_untaken_messages_is_ended_too() {
    // 1 MiB messages: past the bytes long before the count, and read
    // faster than a browser taking one each 200 ms.
    let text = Some("x".repeat(1 << 20));
    let frame = ipc::frame(&ToBrowser::Copied {
        view: 0,
        text,
        cut: false,
    })
    .expect("short");
    let (gone, killed, _) = flooded(vec![frame; 100], Taking::Slowly(200));
    assert!(killed && past_its_budget(&gone), "{gone:?}");
}

#[test]
fn a_renderer_waits_for_a_stalled_browser_and_is_not_ended() {
    // The lead's review, 2026-11-26: a browser held in a modal loop (a
    // Save dialog, a window dragged) takes nothing; a healthy renderer's
    // messages past the budget wait, its pipe filling, and all arrive
    // once the browser takes again.
    let frame = ipc::frame(&ToBrowser::FocusLeft { view: 0 }).expect("short");
    let total = MOST_UNTAKEN_MESSAGES + 2000;
    let (gone, killed, took) = flooded(vec![frame; total], Taking::AfterAStall);
    assert!(!killed && gone.is_empty(), "{gone:?}");
    assert_eq!(took, total);
}

#[test]
fn a_renderer_the_browser_keeps_up_with_is_never_ended_for_its_budget() {
    let frame = ipc::frame(&ToBrowser::FocusLeft { view: 0 }).expect("short");
    let total = MOST_UNTAKEN_MESSAGES + 500;
    let (gone, killed, took) = flooded(vec![frame; total], Taking::Promptly);
    assert_eq!(took, total, "{gone:?}");
    assert!(!killed && gone.is_empty(), "{gone:?}");
}

/// A service that hears a renderer's sound: when it last did.
struct HearsSound(Arc<std::sync::Mutex<Option<Instant>>>);

impl Serve for HearsSound {
    fn serve(
        &self,
        _: u32,
        message: ToBrowser,
        _: &ipc::Sender<ToRenderer>,
    ) -> Result<Option<ToBrowser>, String> {
        if let ToBrowser::Audio { .. } = message {
            *self.0.lock().expect("not poisoned") = Some(Instant::now());
            return Ok(None);
        }
        Ok(Some(message))
    }
}

#[test]
fn a_stalled_browser_still_serves_a_renderers_sound_at_once() {
    // The lead's review, 2026-11-26: messages past a stalled browser's
    // budget are parked, never the reading: a page's music plays on, and
    // the GPU answers, through a minute of a Save dialog.
    let (events, received) = mpsc::channel();
    let killed = Arc::new(AtomicBool::new(false));
    let heard = Arc::new(std::sync::Mutex::new(None));
    let sent = Arc::new(std::sync::Mutex::new(None));
    let total = MOST_UNTAKEN_MESSAGES + 2000;
    let wrote = Arc::clone(&sent);
    let fake = Fake(
        move |mut input: ipc::PipeReader, mut output: ipc::PipeWriter| {
            let frame = ipc::frame(&ToBrowser::FocusLeft { view: 0 }).expect("short");
            let sound = ipc::frame(&ToBrowser::Audio {
                samples: ipc::Bytes(vec![0; 4096]),
            })
            .expect("short");
            let _ = output.write_all(&hello(IPC_VERSION));
            for _ in 0..total {
                if output.write_all(&frame).is_err() {
                    return;
                }
            }
            *wrote.lock().expect("not poisoned") = Some(Instant::now());
            let _ = output.write_all(&sound);
            let mut sink = Vec::new();
            let _ = input.read_to_end(&mut sink);
        },
        killed.clone(),
    );
    let serve: Arc<dyn Serve> = Arc::new(HearsSound(Arc::clone(&heard)));
    let renderer = start_within(&fake, 9, events, nothing(), Some(serve), WAIT).expect("started");
    // The browser stalled: nothing taken for two seconds.
    thread::sleep(Duration::from_secs(2));
    // Served while the browser has taken nothing yet: the order says so,
    // not the time it took (the timing audit, 2026-11-26).
    let sent_at = sent
        .lock()
        .expect("not poisoned")
        .expect("the sound was sent");
    let heard_at = heard
        .lock()
        .expect("not poisoned")
        .expect("the sound was served while the browser was stalled");
    assert!(heard_at >= sent_at, "served before it was sent");
    // Taken again, every parked message arrives, and nothing was ended.
    let mut took = 0;
    while took < total {
        match received.recv_timeout(WAIT).expect("an event in time") {
            (_, Event::Message(..)) => took += 1,
            (_, Event::Gone(why)) => panic!("ended: {why:?}"),
            (_, Event::Ready) => {}
        }
    }
    assert!(!killed.load(Ordering::Relaxed));
    drop(renderer);
}

#[test]
fn a_stalled_browsers_parked_console_is_merged_and_kept_whole() {
    let (events, received) = mpsc::channel();
    let killed = Arc::new(AtomicBool::new(false));
    let total = MOST_UNTAKEN_MESSAGES + 2000;
    let fake = Fake(
        move |mut input: ipc::PipeReader, mut output: ipc::PipeWriter| {
            let _ = output.write_all(&hello(IPC_VERSION));
            for n in 0..total {
                let entry = crate::ConsoleEntry {
                    text: n.to_string(),
                    ..Default::default()
                };
                let frame = ipc::frame(&ToBrowser::Console {
                    view: 0,
                    entries: vec![entry],
                })
                .expect("short");
                if output.write_all(&frame).is_err() {
                    return;
                }
            }
            let mut sink = Vec::new();
            let _ = input.read_to_end(&mut sink);
        },
        killed.clone(),
    );
    let renderer = start_within(&fake, 9, events, nothing(), None, WAIT).expect("started");
    thread::sleep(Duration::from_secs(2));
    let (mut messages, mut entries) = (0, Vec::new());
    while entries.len() < total {
        match received.recv_timeout(WAIT).expect("an event in time") {
            (_, Event::Message(message, _)) => {
                messages += 1;
                if let ToBrowser::Console { entries: more, .. } = *message {
                    entries.extend(more.into_iter().map(|entry| entry.text));
                }
            }
            (_, Event::Gone(why)) => panic!("ended: {why:?}"),
            (_, Event::Ready) => {}
        }
    }
    assert!(
        messages < total,
        "{messages} messages: the parked ones merged"
    );
    let wanted: Vec<String> = (0..total).map(|n| n.to_string()).collect();
    assert_eq!(entries, wanted, "every entry, in order");
    assert!(!killed.load(Ordering::Relaxed));
    drop(renderer);
}

#[test]
fn a_renderer_silent_after_a_burst_has_every_parked_message_arrive() {
    // The lead's review, 2026-11-26: a burst past a stalled browser's
    // budget, then nothing more from the renderer, ever: what was parked
    // arrives as the browser takes, no message waiting on a next one.
    let (events, received) = mpsc::channel();
    let killed = Arc::new(AtomicBool::new(false));
    let total = MOST_UNTAKEN_MESSAGES + 3000;
    let fake = Fake(
        move |mut input: ipc::PipeReader, mut output: ipc::PipeWriter| {
            let frame = ipc::frame(&ToBrowser::FocusLeft { view: 0 }).expect("short");
            let _ = output.write_all(&hello(IPC_VERSION));
            for _ in 0..total {
                if output.write_all(&frame).is_err() {
                    return;
                }
            }
            // Silent from now on.
            let mut sink = Vec::new();
            let _ = input.read_to_end(&mut sink);
        },
        killed.clone(),
    );
    let renderer = start_within(&fake, 9, events, nothing(), None, WAIT).expect("started");
    thread::sleep(Duration::from_secs(2));
    let mut took = 0;
    let mut last = Instant::now();
    let mut longest = Duration::ZERO;
    while took < total {
        let (_, event) = received
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|_| panic!("{took} of {total}: the rest stayed parked"));
        match event {
            Event::Message(..) => took += 1,
            Event::Gone(why) => panic!("ended: {why:?}"),
            Event::Ready => {}
        }
        longest = longest.max(last.elapsed());
        last = Instant::now();
    }
    assert!(
        longest < Duration::from_millis(300),
        "a message waited {longest:?} after the one before"
    );
    assert!(!killed.load(Ordering::Relaxed));
    drop(renderer);
}
