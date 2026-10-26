//! The browser's relay to the GPU process (M41 step 1, D38): it starts
//! the GPU process at a renderer's first request, forwards each
//! renderer's batches stamped with the renderer's number, which it
//! alone gives, and each reply to its renderer. It reads nothing else of
//! them. A renderer the GPU process refused is refused its next message,
//! and so killed; a GPU process that ends loses every renderer's GPU,
//! and another starts at the next request, [`MAX_GPU_RESTARTS`] times in
//! [`RESTART_WINDOW`], after which the GPU is off for the session.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::process::{HELLO_TIMEOUT, Launcher, RendererProcess, Serve};
use crate::gpu::wire::{self, FromGpu, GPU_VERSION, Reply, ToGpu};
use crate::ipc::{self, Bytes, Sender, ToBrowser, ToRenderer};

/// How many times the GPU process may start in [`RESTART_WINDOW`].
pub const MAX_GPU_RESTARTS: usize = 3;

/// The window [`MAX_GPU_RESTARTS`] counts starts in.
pub const RESTART_WINDOW: Duration = Duration::from_secs(5 * 60);

/// The GPU process, while it runs.
struct Running {
    sender: Sender<ToGpu>,
    process: Box<dyn RendererProcess>,
    /// Its number among the starts, so that its reader's end is told
    /// apart from a later process's.
    generation: u64,
}

#[derive(Default)]
struct State {
    running: Option<Running>,
    generation: u64,
    /// When it last started, the latest last.
    starts: VecDeque<Instant>,
    /// Why the GPU is off for the session, once it is.
    off: Option<String>,
    /// Where each renderer that asked hears its replies.
    renderers: HashMap<u32, Sender<ToRenderer>>,
    /// The renderers the GPU process refused, and why.
    refused: HashMap<u32, String>,
    /// The renderers whose GPU was lost and who have not asked since.
    lost: HashSet<u32>,
}

pub struct GpuRelay {
    launcher: Arc<dyn Launcher>,
    fallback_only: bool,
    state: Arc<Mutex<State>>,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A batch of one [`Reply::Lost`], as the GPU process would send it.
fn lost(why: &str) -> ToRenderer {
    ToRenderer::Gpu {
        bytes: Bytes(wire::replies(&[Reply::Lost {
            why: why.to_owned(),
        }])),
    }
}

impl GpuRelay {
    pub fn new(launcher: Arc<dyn Launcher>, fallback_only: bool) -> Arc<GpuRelay> {
        Arc::new(GpuRelay {
            launcher,
            fallback_only,
            state: Arc::default(),
        })
    }

    /// Renderer `renderer` is gone: its objects in the GPU process go.
    pub fn renderer_gone(&self, renderer: u32) {
        let mut state = lock(&self.state);
        state.renderers.remove(&renderer);
        state.refused.remove(&renderer);
        state.lost.remove(&renderer);
        if let Some(running) = &state.running {
            let _ = running.sender.send(&ToGpu::Gone { client: renderer });
        }
    }

    /// Whether the GPU process runs.
    pub fn running(&self) -> bool {
        lock(&self.state).running.is_some()
    }

    /// Ends the GPU process, as a crash would (the tests').
    pub fn kill(&self) {
        if let Some(running) = lock(&self.state).running.as_mut() {
            running.process.kill();
        }
    }

    /// The GPU process, started if it does not run: its sender, or why
    /// there is none.
    fn sender(&self, state: &mut State) -> Result<Sender<ToGpu>, String> {
        if let Some(running) = &state.running {
            return Ok(running.sender.clone());
        }
        if let Some(why) = &state.off {
            return Err(why.clone());
        }
        let now = Instant::now();
        while state
            .starts
            .front()
            .is_some_and(|start| now.duration_since(*start) > RESTART_WINDOW)
        {
            state.starts.pop_front();
        }
        if state.starts.len() >= MAX_GPU_RESTARTS {
            let why = format!("the GPU process ended {MAX_GPU_RESTARTS} times");
            state.off = Some(why.clone());
            return Err(why);
        }
        state.starts.push_back(now);
        let launched = self
            .launcher
            .launch_gpu()
            .map_err(|error| format!("no GPU process: {error}"))?;
        let sender: Sender<ToGpu> = Sender::new(launched.to_renderer);
        let mut process = launched.process;
        state.generation += 1;
        let generation = state.generation;
        let (hello, heard) = std::sync::mpsc::channel();
        let shared = self.state.clone();
        let ended = self.state.clone();
        let read = ipc::spawn_reader::<FromGpu, _>(
            "gpu-relay",
            launched.from_renderer,
            {
                let mut said_hello = false;
                move |message| {
                    if !said_hello {
                        said_hello = true;
                        let _ = hello.send(matches!(
                            message,
                            FromGpu::Hello { version } if version == GPU_VERSION
                        ));
                        return;
                    }
                    deliver(&shared, message);
                }
            },
            move |_| end(&ended, generation),
        );
        if let Err(error) = read {
            process.kill();
            return Err(format!("no reader of the GPU process: {error}"));
        }
        let _ = sender.send(&ToGpu::Hello {
            version: GPU_VERSION,
            fallback_only: self.fallback_only,
        });
        if heard.recv_timeout(HELLO_TIMEOUT) != Ok(true) {
            process.kill();
            return Err("the GPU process did not answer its hello".to_owned());
        }
        state.running = Some(Running {
            sender: sender.clone(),
            process,
            generation,
        });
        Ok(sender)
    }
}

/// A message of the GPU process's, for its renderer.
fn deliver(state: &Mutex<State>, message: FromGpu) {
    let mut state = lock(state);
    match message {
        FromGpu::Client { client, bytes } => {
            if let Some(renderer) = state.renderers.get(&client) {
                let _ = renderer.send(&ToRenderer::Gpu { bytes });
            }
        }
        FromGpu::Bad { client, why } => {
            if let Some(renderer) = state.renderers.get(&client) {
                let _ = renderer.send(&lost(&why));
            }
            state.refused.insert(client, why);
        }
        // A second hello is no reply to anyone.
        FromGpu::Hello { .. } => {}
    }
}

/// The GPU process of `generation` ended: every renderer that asked
/// loses its GPU.
fn end(state: &Mutex<State>, generation: u64) {
    let mut state = lock(state);
    if state
        .running
        .as_ref()
        .is_none_or(|running| running.generation != generation)
    {
        return;
    }
    state.running = None;
    let numbers: Vec<u32> = state.renderers.keys().copied().collect();
    for number in numbers {
        if let Some(renderer) = state.renderers.get(&number) {
            let _ = renderer.send(&lost("the GPU process ended"));
        }
        state.lost.insert(number);
    }
}

impl Serve for GpuRelay {
    fn serve(
        &self,
        renderer: u32,
        message: ToBrowser,
        reply: &Sender<ToRenderer>,
    ) -> Result<Option<ToBrowser>, String> {
        let ToBrowser::Gpu { bytes } = message else {
            return Ok(Some(message));
        };
        let mut state = lock(&self.state);
        if let Some(why) = state.refused.get(&renderer) {
            return Err(format!("the GPU process refused it: {why}"));
        }
        // A renderer that lost its GPU starts again with nothing: what it
        // asks of objects it had is refused by the new process.
        state.lost.remove(&renderer);
        state.renderers.insert(renderer, reply.clone());
        let sender = match self.sender(&mut state) {
            Ok(sender) => sender,
            Err(why) => {
                let _ = reply.send(&lost(&why));
                return Ok(None);
            }
        };
        if sender
            .send(&ToGpu::Client {
                client: renderer,
                bytes,
            })
            .is_err()
        {
            let _ = reply.send(&lost("the GPU process ended"));
        }
        Ok(None)
    }
}
