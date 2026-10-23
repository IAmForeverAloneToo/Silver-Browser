//! The renderers' sound in the browser (M40 step 4): a renderer's blocks
//! reach the output through the service, made safe to play, and the
//! renderer hears what was played; a block no renderer may send kills
//! it.

use std::sync::Arc;

use super::AudioService;
use super::process::Serve;
use crate::audio_output::MAX_AUDIO_BLOCK_FRAMES;
use crate::ipc::{Bytes, Sender, ToBrowser, ToRenderer, pipe, read_message};
use crate::renderer::BrowserSink;
use crate::renderer::link::Link;
use crate::testing::Capture;
use crate::{AudioOutput, AudioSink};

/// An output whose every source is one capture.
struct Captured(Arc<Capture>);

impl AudioOutput for Captured {
    fn source(&self) -> Arc<dyn AudioSink> {
        self.0.clone()
    }
}

#[test]
fn a_renderers_sound_reaches_the_output_and_it_hears_what_was_played() {
    let capture = Arc::new(Capture::default());
    let service = AudioService::new(Arc::new(Captured(capture.clone())));
    let (to_browser, mut from_renderer) = pipe();
    let link = Link::new(Sender::new(to_browser));
    let sink = BrowserSink(link.clone());
    sink.write(&[0.5, -0.5, 2.0, f32::NAN, -3.0, 0.25]);
    let message: ToBrowser = read_message(&mut from_renderer).unwrap().unwrap();
    let (to_renderer, mut from_browser) = pipe();
    let reply = Sender::new(to_renderer);
    assert!(service.serve(7, message, &reply).unwrap().is_none());
    assert_eq!(
        *capture.samples.lock().unwrap(),
        [0.5, -0.5, 1.0, 0.0, -1.0, 0.25]
    );
    let played: ToRenderer = read_message(&mut from_browser).unwrap().unwrap();
    assert!(
        matches!(played, ToRenderer::AudioPlayed { frames: 3 }),
        "{played:?}"
    );
    assert!(link.route(played).unwrap().is_none());
    assert_eq!(sink.played(), 3);
    // An empty block asks, and is answered.
    sink.write(&[]);
    let ask: ToBrowser = read_message(&mut from_renderer).unwrap().unwrap();
    assert!(service.serve(7, ask, &reply).unwrap().is_none());
    let again: ToRenderer = read_message(&mut from_browser).unwrap().unwrap();
    assert!(
        matches!(again, ToRenderer::AudioPlayed { frames: 3 }),
        "{again:?}"
    );
}

#[test]
fn a_block_no_renderer_may_send_is_refused() {
    let service = AudioService::new(Arc::new(Captured(Arc::default())));
    let (to_renderer, _from_browser) = pipe();
    let reply = Sender::new(to_renderer);
    let block = |bytes: usize| ToBrowser::Audio {
        samples: Bytes(vec![0; bytes]),
    };
    // Half a frame.
    assert!(service.serve(1, block(12), &reply).is_err());
    // Longer than a block may be.
    assert!(
        service
            .serve(1, block((MAX_AUDIO_BLOCK_FRAMES + 1) * 8), &reply)
            .is_err()
    );
    assert!(
        service
            .serve(1, block(MAX_AUDIO_BLOCK_FRAMES * 8), &reply)
            .unwrap()
            .is_none()
    );
    // Not the service's.
    let other = ToBrowser::CancelFetch { id: 1 };
    assert!(matches!(
        service.serve(1, other, &reply).unwrap(),
        Some(ToBrowser::CancelFetch { .. })
    ));
}
