//! A client of the GPU service (M41 step 1, D38): what a renderer, or the
//! browser's engines in one process, hold to reach it. It numbers the
//! objects its documents make and the calls they wait on, sends their
//! requests in batches, and hands each reply to the one who asked; when
//! its GPU is lost, everyone waiting hears it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::wire::{self, Reply, Request};

/// Where a client's batches go: the browser's relay, or the local
/// service's thread.
pub type Channel = Box<dyn Fn(Vec<u8>) + Send + Sync>;

/// One who waits on a call's reply.
type Answer = Box<dyn FnOnce(Reply) + Send>;

pub struct GpuClient {
    channel: Channel,
    next_id: AtomicU32,
    next_call: AtomicU32,
    waiting: Mutex<HashMap<u32, Answer>>,
    /// Why its GPU was lost, once it was.
    lost: Mutex<Option<String>>,
}

impl GpuClient {
    pub fn new(channel: Channel) -> GpuClient {
        GpuClient {
            channel,
            next_id: AtomicU32::new(1),
            next_call: AtomicU32::new(1),
            waiting: Mutex::default(),
            lost: Mutex::default(),
        }
    }

    fn waiting(&self) -> MutexGuard<'_, HashMap<u32, Answer>> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A number for a new object, unique among this client's.
    pub fn id(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Sends `requests` as one batch.
    pub fn send(&self, requests: &[Request]) {
        if !requests.is_empty() && !self.is_lost() {
            (self.channel)(wire::requests(requests));
        }
    }

    /// Sends the request `ask` makes of a new call's number; `answer`
    /// hears its reply, or [`Reply::Lost`] if the GPU is lost first.
    pub fn call(
        &self,
        ask: impl FnOnce(u32) -> Request,
        answer: impl FnOnce(Reply) + Send + 'static,
    ) {
        if let Some(why) = self.lost() {
            answer(Reply::Lost { why });
            return;
        }
        let call = self.next_call.fetch_add(1, Ordering::Relaxed);
        self.waiting().insert(call, Box::new(answer));
        self.send(&[ask(call)]);
    }

    /// Takes a batch of replies from the GPU service. Err for bytes that
    /// are no batch, or a reply to no call: the service does not send
    /// those.
    pub fn deliver(&self, bytes: &[u8]) -> Result<(), String> {
        let replies = wire::read_replies(bytes)
            .map_err(|malformed| format!("a malformed batch: {malformed}"))?;
        for reply in replies {
            match &reply {
                Reply::Adapter { call, .. } => {
                    let Some(answer) = self.waiting().remove(call) else {
                        return Err(format!("a reply to no call ({call})"));
                    };
                    answer(reply);
                }
                Reply::Lost { why } => self.lose(why.clone()),
            }
        }
        Ok(())
    }

    /// Its GPU is lost: everyone waiting hears so, and nothing more is
    /// sent.
    pub fn lose(&self, why: String) {
        *self.lost.lock().unwrap_or_else(PoisonError::into_inner) = Some(why.clone());
        let waiting: Vec<Answer> = self.waiting().drain().map(|(_, answer)| answer).collect();
        for answer in waiting {
            answer(Reply::Lost { why: why.clone() });
        }
    }

    pub fn is_lost(&self) -> bool {
        self.lost().is_some()
    }

    pub fn lost(&self) -> Option<String> {
        self.lost
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
