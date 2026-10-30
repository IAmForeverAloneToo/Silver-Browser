//! Sound from outside the graph (a `MediaElementAudioSourceNode`'s, or a
//! `MediaStreamAudioSourceNode`'s): the element's pipeline or another
//! graph's tap puts frames in, the graph takes a quantum at a time,
//! silence when none came.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, Weak};

use crate::QUANTUM;
use crate::mix::{Bus, silence};

/// The most frames of a stream kept waiting for its reader: a mixer's
/// wake's worth twice over.
pub const MAX_STREAM_WAITING: usize = 16_384;

/// The most frames kept waiting: past it, the oldest are dropped.
const MAX_WAITING: usize = 48_000;

/// The frames a feed gathers before it is read, and again after it ran
/// dry: what comes at another pace than the graph takes it (an element's
/// sound resampled, another graph's) then plays without gaps.
pub const PRIME: usize = 4 * QUANTUM;

/// Frames waiting, by channel.
#[derive(Default)]
pub struct Feed {
    pub channels: Vec<VecDeque<f32>>,
    /// Whether it gathered [`PRIME`] frames since it last ran dry.
    primed: bool,
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

impl Feed {
    /// Adds a quantum of `bus`, channel by channel, keeping at most
    /// [`MAX_STREAM_WAITING`] frames: a reader that stopped a while
    /// takes up what is new, not what is old.
    pub fn push_bus(&mut self, bus: &Bus) {
        let channels = bus.len().max(1);
        if self.channels.len() != channels {
            self.channels = vec![VecDeque::new(); channels];
        }
        for (queue, channel) in self.channels.iter_mut().zip(bus) {
            queue.extend(channel.iter().copied());
            let excess = queue.len().saturating_sub(MAX_STREAM_WAITING);
            queue.drain(..excess);
        }
    }
}

#[derive(Clone, Default)]
pub struct External {
    pub feed: Arc<Mutex<Feed>>,
    /// The rate of the graph that takes it, which what is put in is to
    /// be at; 0 for the output's.
    pub rate: f32,
}

/// Sound going out of the graph (a `MediaStreamAudioDestinationNode`'s
/// track): each quantum its node takes is given to every feed reading
/// it, until the track stops.
#[derive(Clone, Default)]
pub struct Tap {
    pub readers: Arc<Mutex<TapReaders>>,
}

/// The feeds reading a [`Tap`], and whether its track stopped.
#[derive(Default)]
pub struct TapReaders {
    pub feeds: Vec<Weak<Mutex<Feed>>>,
    pub stopped: bool,
}

/// The most feeds one tap gives to.
pub const MAX_READERS: usize = 64;

impl Tap {
    /// A feed reading the tap from now: silence once its track stopped.
    pub fn reader(&self) -> External {
        let external = External::default();
        if let Ok(mut readers) = self.readers.lock()
            && !readers.stopped
        {
            readers.feeds.retain(|feed| feed.strong_count() > 0);
            if readers.feeds.len() < MAX_READERS {
                readers.feeds.push(Arc::downgrade(&external.feed));
            }
        }
        external
    }

    /// Stops the track: no feed reads it again.
    pub fn stop(&self) {
        if let Ok(mut readers) = self.readers.lock() {
            readers.stopped = true;
            readers.feeds.clear();
        }
    }

    pub(crate) fn give(&self, bus: &Bus) {
        let Ok(readers) = self.readers.lock() else {
            return;
        };
        for feed in readers.feeds.iter().filter_map(Weak::upgrade) {
            if let Ok(mut feed) = feed.lock() {
                feed.push_bus(bus);
            }
        }
    }
}

impl External {
    pub(crate) fn take(&self) -> Bus {
        let Ok(mut feed) = self.feed.lock() else {
            return silence(1);
        };
        if feed.channels.is_empty() {
            return silence(1);
        }
        let waiting = feed.channels.first().map_or(0, VecDeque::len);
        if !feed.primed && waiting < PRIME {
            return silence(feed.channels.len());
        }
        feed.primed = waiting >= QUANTUM;
        let mut out = silence(feed.channels.len());
        for (channel, queue) in out.iter_mut().zip(&mut feed.channels) {
            for slot in channel.iter_mut().take(QUANTUM) {
                *slot = queue.pop_front().unwrap_or(0.0);
            }
        }
        out
    }
}
