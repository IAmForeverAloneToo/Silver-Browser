//! A client of the GPU service (M41 step 1, D38): what a renderer, or the
//! browser's engines in one process, hold to reach it. It numbers the
//! objects its documents make and the calls they wait on, queues their
//! requests and sends them in batches, and hands each reply to the one
//! who asked; a device's own news (an uncaptured error, its loss) to the
//! one listening to it. When its GPU is lost, every call waiting and
//! every device hears it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::wire::{self, GPU_BATCH, GPU_CHUNK, Reply, Request};
use crate::ipc::Bytes;

/// Where a client's batches go: the browser's relay, or the local
/// service's thread.
pub type Channel = Box<dyn Fn(Vec<u8>) + Send + Sync>;

/// One who waits on a call's reply, and the bytes a map gathered.
type Answer = Box<dyn FnOnce(Reply, Vec<u8>) + Send>;

/// One who listens to a device's news.
type Listener = Box<dyn Fn(Reply) + Send + Sync>;

#[derive(Default)]
struct State {
    waiting: HashMap<u32, Answer>,
    /// The bytes of each map under way, gathered from its pieces.
    gathered: HashMap<u32, Vec<u8>>,
    listening: HashMap<u32, Listener>,
    /// Requests not yet sent, and their bytes.
    queued: Vec<Request>,
    queued_bytes: usize,
    /// Why its GPU was lost, once it was.
    lost: Option<String>,
}

pub struct GpuClient {
    channel: Channel,
    next_id: AtomicU32,
    next_call: AtomicU32,
    state: Mutex<State>,
}

impl GpuClient {
    pub fn new(channel: Channel) -> GpuClient {
        GpuClient {
            channel,
            next_id: AtomicU32::new(1),
            next_call: AtomicU32::new(1),
            state: Mutex::default(),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A number for a new object, unique among this client's.
    pub fn id(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Queues `requests`, sending what was queued once it makes a batch.
    pub fn queue(&self, requests: impl IntoIterator<Item = Request>) {
        let full = {
            let mut state = self.state();
            if state.lost.is_some() {
                return;
            }
            for request in requests {
                state.queued_bytes += crate::ipc::encode(&request).len();
                state.queued.push(request);
            }
            state.queued_bytes >= GPU_BATCH
        };
        if full {
            self.flush();
        }
    }

    /// Queues the requests `write` makes of `bytes` in pieces of at most
    /// [`GPU_CHUNK`], each with its offset from `offset`.
    pub fn queue_bytes(&self, offset: u64, bytes: &[u8], write: impl Fn(u64, Bytes) -> Request) {
        for (i, piece) in bytes.chunks(GPU_CHUNK).enumerate() {
            self.queue([write(
                offset + (i * GPU_CHUNK) as u64,
                Bytes(piece.to_vec()),
            )]);
        }
    }

    /// Sends what was queued.
    pub fn flush(&self) {
        let batch = {
            let mut state = self.state();
            state.queued_bytes = 0;
            std::mem::take(&mut state.queued)
        };
        if !batch.is_empty() {
            (self.channel)(wire::requests(&batch));
        }
    }

    /// Sends `requests` as one batch, after what was queued.
    pub fn send(&self, requests: &[Request]) {
        self.queue(requests.iter().cloned());
        self.flush();
    }

    /// Queues the request `ask` makes of a new call's number; `answer`
    /// hears its reply (with the bytes of a map), or [`Reply::Lost`] if
    /// the GPU is lost first.
    pub fn call(
        &self,
        ask: impl FnOnce(u32) -> Request,
        answer: impl FnOnce(Reply, Vec<u8>) + Send + 'static,
    ) {
        let mut state = self.state();
        if let Some(why) = state.lost.clone() {
            drop(state);
            answer(Reply::Lost { why }, Vec::new());
            return;
        }
        let call = self.next_call.fetch_add(1, Ordering::Relaxed);
        state.waiting.insert(call, Box::new(answer));
        drop(state);
        self.queue([ask(call)]);
    }

    /// Hands device `device`'s news (uncaptured errors, its loss) to
    /// `listener` from now.
    pub fn listen(&self, device: u32, listener: impl Fn(Reply) + Send + Sync + 'static) {
        self.state().listening.insert(device, Box::new(listener));
    }

    /// No one listens to device `device` any more.
    pub fn unlisten(&self, device: u32) {
        self.state().listening.remove(&device);
    }

    /// Takes a batch of replies from the GPU service. Err for bytes that
    /// are no batch, or a reply to no call: the service does not send
    /// those.
    pub fn deliver(&self, bytes: &[u8]) -> Result<(), String> {
        let replies = wire::read_replies(bytes)
            .map_err(|malformed| format!("a malformed batch: {malformed}"))?;
        for reply in replies {
            let call = match &reply {
                Reply::Adapter { call, .. }
                | Reply::Device { call, .. }
                | Reply::ErrorScope { call, .. }
                | Reply::Mapped { call, .. }
                | Reply::WorkDone { call }
                | Reply::Compilation { call, .. }
                | Reply::PipelineMade { call, .. } => *call,
                Reply::MappedBytes {
                    call,
                    offset,
                    bytes,
                } => {
                    let mut state = self.state();
                    if !state.waiting.contains_key(call) {
                        return Err(format!("bytes for no call ({call})"));
                    }
                    let gathered = state.gathered.entry(*call).or_default();
                    if *offset != gathered.len() as u64 {
                        return Err(format!("a piece out of order ({call})"));
                    }
                    gathered.extend_from_slice(&bytes.0);
                    continue;
                }
                Reply::Uncaptured { device, .. } | Reply::DeviceLost { device, .. } => {
                    let state = self.state();
                    if let Some(listener) = state.listening.get(device) {
                        listener(reply.clone());
                    }
                    continue;
                }
                Reply::Lost { why } => {
                    self.lose(why.clone());
                    continue;
                }
            };
            let (answer, bytes) = {
                let mut state = self.state();
                let Some(answer) = state.waiting.remove(&call) else {
                    return Err(format!("a reply to no call ({call})"));
                };
                (answer, state.gathered.remove(&call).unwrap_or_default())
            };
            answer(reply, bytes);
        }
        Ok(())
    }

    /// Its GPU is lost: everyone waiting hears so, every device listened
    /// to is lost, and nothing more is sent.
    pub fn lose(&self, why: String) {
        let (waiting, listening) = {
            let mut state = self.state();
            if state.lost.is_some() {
                return;
            }
            state.lost = Some(why.clone());
            state.queued.clear();
            state.gathered.clear();
            let waiting: Vec<Answer> = state.waiting.drain().map(|(_, answer)| answer).collect();
            let listening: Vec<(u32, Listener)> = state.listening.drain().collect();
            (waiting, listening)
        };
        for answer in waiting {
            answer(Reply::Lost { why: why.clone() }, Vec::new());
        }
        for (device, listener) in listening {
            listener(Reply::DeviceLost {
                device,
                destroyed: false,
                message: why.clone(),
            });
        }
    }

    pub fn is_lost(&self) -> bool {
        self.lost().is_some()
    }

    pub fn lost(&self) -> Option<String> {
        self.state().lost.clone()
    }
}
