//! The renderers' sound in the browser (M40 step 4, D37): each
//! renderer's blocks go to a source of the browser's audio output, and
//! the renderer hears back what was played, by which its mixer keeps
//! time. A block is checked as a renderer's lie may be: its length, and
//! samples made safe to play.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::process::Serve;
use crate::audio_output::{MAX_AUDIO_BLOCK_FRAMES, OUTPUT_CHANNELS, clamp_samples};
use crate::ipc::{Sender, ToBrowser, ToRenderer};
use crate::{AudioOutput, AudioSink};

/// A renderer's source silent this long is let go.
const IDLE: Duration = Duration::from_secs(10);

/// A renderer's source, and when it last sent sound.
type Source = (Arc<dyn AudioSink>, Instant);

pub struct AudioService {
    output: Arc<dyn AudioOutput>,
    sources: Mutex<HashMap<u32, Source>>,
}

impl AudioService {
    pub fn new(output: Arc<dyn AudioOutput>) -> Arc<AudioService> {
        Arc::new(AudioService {
            output,
            sources: Mutex::default(),
        })
    }
}

/// Little-endian `f32` bytes as samples.
fn floats(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect()
}

impl Serve for AudioService {
    fn serve(
        &self,
        renderer: u32,
        message: ToBrowser,
        reply: &Sender<ToRenderer>,
    ) -> Result<Option<ToBrowser>, String> {
        let ToBrowser::Audio { samples } = message else {
            return Ok(Some(message));
        };
        let frame = 4 * OUTPUT_CHANNELS;
        if samples.0.len() % frame != 0 || samples.0.len() / frame > MAX_AUDIO_BLOCK_FRAMES {
            return Err(format!("a block of sound of {} bytes", samples.0.len()));
        }
        let now = Instant::now();
        let source = {
            let mut sources = self.sources.lock().unwrap_or_else(PoisonError::into_inner);
            sources.retain(|key, (_, last)| *key == renderer || now.duration_since(*last) < IDLE);
            let entry = sources
                .entry(renderer)
                .or_insert_with(|| (self.output.source(), now));
            entry.1 = now;
            entry.0.clone()
        };
        if !samples.0.is_empty() {
            let mut block = floats(&samples.0);
            clamp_samples(&mut block);
            source.write(&block);
        }
        let _ = reply.send(&ToRenderer::AudioPlayed {
            frames: source.played(),
        });
        Ok(None)
    }
}
