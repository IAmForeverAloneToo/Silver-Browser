//! The browser's relay to the GPU process (M41 step 1, D38), over a GPU
//! process run as a thread.

use std::io::BufReader;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};

use super::process::{Launched, Launcher, Serve, ThreadLauncher};
use super::{GpuRelay, MAX_GPU_RESTARTS};
use crate::gpu::wire::{self, Reply, Request};
use crate::ipc::{self, Bytes, PipeReader, Sender, ToBrowser, ToRenderer};

/// A launcher of GPU processes as threads, counting them.
#[derive(Default)]
struct Counting {
    threads: ThreadLauncher,
    started: AtomicUsize,
}

impl Launcher for Counting {
    fn launch(&self, number: u32) -> std::io::Result<Launched> {
        self.threads.launch(number)
    }

    fn launch_gpu(&self) -> std::io::Result<Launched> {
        self.started.fetch_add(1, Ordering::Relaxed);
        self.threads.launch_gpu()
    }
}

/// A renderer as the relay sees it: where its replies go, and what it
/// hears.
struct Renderer {
    number: u32,
    sender: Sender<ToRenderer>,
    heard: mpsc::Receiver<ToRenderer>,
}

impl Renderer {
    fn new(number: u32) -> Renderer {
        let (writer, reader) = ipc::pipe();
        // What it hears, read on a thread, so that a reply that never
        // comes fails the test rather than holding it.
        let (heard, hear) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader: BufReader<PipeReader> = BufReader::new(reader);
            while let Ok(Some(message)) = ipc::read_message::<ToRenderer>(&mut reader) {
                let _ = heard.send(message);
            }
        });
        Renderer {
            number,
            sender: Sender::new(writer),
            heard: hear,
        }
    }

    /// Sends `requests` through the relay.
    fn ask(&self, relay: &GpuRelay, requests: &[Request]) -> Result<Option<ToBrowser>, String> {
        let message = ToBrowser::Gpu {
            bytes: Bytes(wire::requests(requests)),
        };
        relay.serve(self.number, message, &self.sender)
    }

    /// The replies of the next batch it hears.
    fn hear(&mut self) -> Vec<Reply> {
        match self.heard.recv_timeout(std::time::Duration::from_secs(30)) {
            Ok(ToRenderer::Gpu { bytes }) => wire::read_replies(&bytes.0).expect("a batch"),
            other => panic!("not a batch: {other:?}"),
        }
    }
}

fn adapter(call: u32, id: u32) -> Request {
    Request::RequestAdapter {
        call,
        id,
        power: 0,
        fallback: false,
    }
}

fn relay() -> (Arc<GpuRelay>, Arc<Counting>) {
    let launcher = Arc::new(Counting::default());
    (GpuRelay::new(launcher.clone(), false), launcher)
}

#[test]
fn the_relay_starts_the_gpu_process_and_carries_each_renderers_batches() {
    let (relay, launcher) = relay();
    assert!(!relay.running());
    let mut first = Renderer::new(1);
    let mut second = Renderer::new(2);
    // Not a GPU message: given back.
    let other = relay.serve(
        1,
        ToBrowser::Audio {
            samples: Bytes(Vec::new()),
        },
        &first.sender,
    );
    assert!(matches!(other, Ok(Some(ToBrowser::Audio { .. }))));
    assert!(!relay.running());
    assert_eq!(first.ask(&relay, &[adapter(1, 1)]), Ok(None));
    assert!(relay.running());
    assert!(matches!(
        &first.hear()[..],
        [Reply::Adapter { call: 1, .. }]
    ));
    // The second renderer's object numbers are its own: the relay stamps
    // each batch with the renderer's number.
    assert_eq!(second.ask(&relay, &[adapter(4, 1)]), Ok(None));
    assert!(matches!(
        &second.hear()[..],
        [Reply::Adapter { call: 4, .. }]
    ));
    assert_eq!(launcher.started.load(Ordering::Relaxed), 1);
}

#[test]
fn a_renderer_the_gpu_process_refuses_loses_its_gpu_and_is_refused_by_the_relay() {
    let (relay, _) = relay();
    let mut renderer = Renderer::new(7);
    renderer.ask(&relay, &[Request::Drop { id: 3 }]).unwrap();
    let [Reply::Lost { why }] = &renderer.hear()[..] else {
        panic!("not lost");
    };
    assert_eq!(why, "object 3 dropped, which it does not have");
    // Its next message is refused, which kills it.
    let refused = renderer.ask(&relay, &[adapter(1, 1)]);
    assert_eq!(
        refused,
        Err("the GPU process refused it: object 3 dropped, which it does not have".to_owned())
    );
    // Another renderer is served.
    let mut other = Renderer::new(8);
    other.ask(&relay, &[adapter(1, 1)]).unwrap();
    assert!(matches!(&other.hear()[..], [Reply::Adapter { .. }]));
    // Gone, its number is forgotten.
    relay.renderer_gone(7);
    let mut again = Renderer::new(7);
    again.ask(&relay, &[adapter(2, 1)]).unwrap();
    assert!(matches!(
        &again.hear()[..],
        [Reply::Adapter { call: 2, .. }]
    ));
}

#[test]
fn a_gpu_process_that_ends_loses_every_renderers_gpu_and_three_ends_turn_it_off() {
    let (relay, launcher) = relay();
    let mut first = Renderer::new(1);
    let mut second = Renderer::new(2);
    for start in 1..=MAX_GPU_RESTARTS {
        first.ask(&relay, &[adapter(1, 1)]).unwrap();
        first.hear();
        second.ask(&relay, &[adapter(1, 1)]).unwrap();
        second.hear();
        assert_eq!(launcher.started.load(Ordering::Relaxed), start);
        relay.kill();
        for renderer in [&mut first, &mut second] {
            let [Reply::Lost { why }] = &renderer.hear()[..] else {
                panic!("not lost");
            };
            assert_eq!(why, "the GPU process ended");
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while relay.running() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    // A fourth start within the window is not made: the GPU is off.
    first.ask(&relay, &[adapter(1, 1)]).unwrap();
    let [Reply::Lost { why }] = &first.hear()[..] else {
        panic!("not lost");
    };
    assert_eq!(
        why,
        &format!("the GPU process ended {MAX_GPU_RESTARTS} times")
    );
    assert_eq!(launcher.started.load(Ordering::Relaxed), MAX_GPU_RESTARTS);
}

#[test]
fn without_a_gpu_process_a_renderer_hears_its_gpu_lost() {
    // A launcher with no GPU process.
    let relay = GpuRelay::new(Arc::new(NoGpu), false);
    let mut renderer = Renderer::new(1);
    renderer.ask(&relay, &[adapter(1, 1)]).unwrap();
    let [Reply::Lost { why }] = &renderer.hear()[..] else {
        panic!("not lost");
    };
    assert!(why.starts_with("no GPU process"), "{why}");
}

struct NoGpu;

impl Launcher for NoGpu {
    fn launch(&self, _: u32) -> std::io::Result<Launched> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
}
