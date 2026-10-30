//! `ScriptProcessorNode` (Web Audio 1.1 section 1.29): its input gathered
//! into buffers of `size` frames, each handed to the page as it fills,
//! and the page's output for a buffer played two buffers later, as the
//! reference browser double-buffers it. A buffer the page has not
//! answered by then plays as silence.

use std::collections::BTreeMap;

use crate::QUANTUM;
use crate::mix::{Bus, silence};

/// The sizes a page may ask for, besides 0: powers of two.
pub const SIZES: [usize; 7] = [256, 512, 1024, 2048, 4096, 8192, 16384];

/// A buffer of input the page is to be given.
#[derive(Debug, Clone, PartialEq)]
pub struct Filled {
    /// Its number, counting from 0.
    pub seq: u64,
    /// The event's `playbackTime`, in frames: the first frame of the
    /// quantum that filled it, a buffer on, as the reference browser
    /// gives it.
    pub plays_at: u64,
    /// Its channels of `size` frames each.
    pub channels: Vec<Vec<f32>>,
}

pub struct Script {
    pub size: usize,
    input_channels: usize,
    output_channels: usize,
    /// The input being gathered, by channel.
    filling: Vec<Vec<f32>>,
    /// The output being played, by channel.
    playing: Vec<Vec<f32>>,
    /// Frames into the current buffer.
    at: usize,
    /// Buffers handed out so far.
    seq: u64,
    /// Buffers filled and not yet taken for the page.
    ready: Vec<Filled>,
    /// The page's outputs not yet played, by buffer.
    answers: BTreeMap<u64, Vec<Vec<f32>>>,
}

impl Script {
    /// A node of buffers of `size` frames, `input_channels` in and
    /// `output_channels` out.
    pub fn new(size: usize, input_channels: usize, output_channels: usize) -> Script {
        let size = size.max(QUANTUM);
        Script {
            size,
            input_channels,
            output_channels,
            filling: vec![vec![0.0; size]; input_channels],
            playing: vec![vec![0.0; size]; output_channels.max(1)],
            at: 0,
            seq: 0,
            ready: Vec::new(),
            answers: BTreeMap::new(),
        }
    }

    /// The buffers filled since this was last asked.
    pub fn take_ready(&mut self) -> Vec<Filled> {
        std::mem::take(&mut self.ready)
    }

    /// The page's output for buffer `seq`: each channel of `size`
    /// frames, those missing silent. One too late to play is dropped.
    pub fn answer(&mut self, seq: u64, mut channels: Vec<Vec<f32>>) {
        // Buffer `seq` plays while buffer `seq + 2` fills.
        if seq + 2 < self.seq + 1 || seq >= self.seq {
            return;
        }
        channels.resize(self.output_channels, Vec::new());
        for channel in &mut channels {
            channel.resize(self.size, 0.0);
        }
        self.answers.insert(seq, channels);
    }

    /// One quantum: `input` gathered, the output played; `frame` is the
    /// quantum's first.
    pub(crate) fn process(&mut self, input: &Bus, frame: u64) -> Bus {
        let at = self.at;
        for (c, channel) in self.filling.iter_mut().enumerate() {
            let from = input.get(c).copied().unwrap_or([0.0; QUANTUM]);
            channel[at..at + QUANTUM].copy_from_slice(&from);
        }
        let mut out = silence(self.output_channels.max(1));
        for (to, from) in out.iter_mut().zip(&self.playing) {
            to.copy_from_slice(&from[at..at + QUANTUM]);
        }
        self.at += QUANTUM;
        if self.at == self.size {
            self.at = 0;
            let filled = std::mem::replace(
                &mut self.filling,
                vec![vec![0.0; self.size]; self.input_channels],
            );
            self.ready.push(Filled {
                seq: self.seq,
                plays_at: frame + self.size as u64,
                channels: filled,
            });
            self.seq += 1;
            // The next buffer plays the page's output for the one filled
            // before the one just handed out.
            let due = self.seq.checked_sub(2);
            self.answers
                .retain(|seq, _| due.is_some_and(|due| *seq >= due));
            self.playing = due
                .and_then(|due| self.answers.remove(&due))
                .unwrap_or_else(|| vec![vec![0.0; self.size]; self.output_channels.max(1)]);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quantum(v: f32) -> Bus {
        vec![[v; QUANTUM]]
    }

    #[test]
    fn a_buffer_is_handed_out_as_it_fills_and_its_answer_plays_two_later() {
        let mut node = Script::new(256, 1, 1);
        let mut frame = 0;
        let mut step = |node: &mut Script, v: f32| {
            let out = node.process(&quantum(v), frame);
            frame += QUANTUM as u64;
            out[0][0]
        };
        assert_eq!(step(&mut node, 1.0), 0.0);
        assert!(node.take_ready().is_empty());
        assert_eq!(step(&mut node, 2.0), 0.0);
        let ready = node.take_ready();
        assert_eq!(ready.len(), 1);
        assert_eq!((ready[0].seq, ready[0].plays_at), (0, 384));
        assert_eq!(ready[0].channels[0][0], 1.0);
        assert_eq!(ready[0].channels[0][255], 2.0);
        node.answer(0, vec![vec![7.0; 256]]);
        // Buffer 1 plays silence: nothing was answered for it.
        assert_eq!(step(&mut node, 0.0), 0.0);
        assert_eq!(step(&mut node, 0.0), 0.0);
        assert_eq!(node.take_ready()[0].seq, 1);
        // Buffer 2 plays buffer 0's answer, from frame 512.
        assert_eq!(step(&mut node, 0.0), 7.0);
        assert_eq!(step(&mut node, 0.0), 7.0);
        // An answer too late to play is dropped.
        node.answer(1, vec![vec![9.0; 256]]);
        assert_eq!(step(&mut node, 0.0), 0.0);
    }
}
