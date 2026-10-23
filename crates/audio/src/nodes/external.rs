//! Sound from outside the graph (a `MediaElementAudioSourceNode`'s): the
//! element's pipeline puts frames in, the graph takes a quantum at a
//! time, silence when none came.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::QUANTUM;
use crate::mix::{Bus, silence};

/// The most frames kept waiting: past it, the oldest are dropped.
const MAX_WAITING: usize = 48_000;

/// Frames waiting, by channel.
#[derive(Default)]
pub struct Feed {
    pub channels: Vec<VecDeque<f32>>,
}

impl Feed {
    /// Adds interleaved `samples` of `channels` channels.
    pub fn push(&mut self, samples: &[f32], channels: usize) {
        let channels = channels.max(1);
        if self.channels.len() != channels {
            self.channels = vec![VecDeque::new(); channels];
        }
        for frame in samples.chunks(channels) {
            for (queue, v) in self.channels.iter_mut().zip(frame) {
                queue.push_back(*v);
            }
        }
        for queue in &mut self.channels {
            let excess = queue.len().saturating_sub(MAX_WAITING);
            queue.drain(..excess);
        }
    }
}

#[derive(Clone, Default)]
pub struct External {
    pub feed: Arc<Mutex<Feed>>,
}

impl External {
    pub(crate) fn take(&self) -> Bus {
        let Ok(mut feed) = self.feed.lock() else {
            return silence(1);
        };
        if feed.channels.is_empty() {
            return silence(1);
        }
        let mut out = silence(feed.channels.len());
        for (channel, queue) in out.iter_mut().zip(&mut feed.channels) {
            for slot in channel.iter_mut().take(QUANTUM) {
                *slot = queue.pop_front().unwrap_or(0.0);
            }
        }
        out
    }
}
