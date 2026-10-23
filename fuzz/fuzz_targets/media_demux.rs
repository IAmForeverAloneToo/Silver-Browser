//! Media (M40) from arbitrary bytes: each container's reader, its tracks'
//! packets decoded by their decoders, a seek, and `decodeAudioData()`'s
//! whole decode. Nothing may panic outside `media`'s guard, a decoder
//! that panicked is refused after, and what decodes must keep its
//! bounds: pictures at most `MAX_VIDEO_SIZE` a side, their pixels exactly
//! their size, opaque; sound finite, a whole number of frames.

#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;
use media::{
    Bytes, Demuxer, Error, Hint, MAX_VIDEO_SIZE, PictureDecoder, SoundDecoder, decode_all,
};

/// The packets read of one input, at most.
const MAX_PACKETS: usize = 2_000;

/// A decoder's or a reader's panic is contained by `media`, as it is in
/// the browser (release builds unwind): the target's hook does not abort
/// on it, as libFuzzer's would, and it comes back as `Error::Panicked`. A
/// panic outside `media`'s guard still ends the run. With
/// `SILVER_FUZZ_PANICS` set to a folder, each input that made one panic
/// is written there.
fn contain_decoder_panics() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| std::panic::set_hook(Box::new(|_| {})));
    // rav1d's panics are never contained: they end the run, as they end
    // the browser's renderer.
    media::abort_on_decoder_panic();
}

fn keep_panicking_input(data: &[u8]) {
    if let Some(folder) = std::env::var_os("SILVER_FUZZ_PANICS") {
        let name = format!(
            "{:016x}",
            data.iter().fold(0xcbf29ce484222325u64, |h, b| {
                (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
            })
        );
        let _ = std::fs::write(std::path::Path::new(&folder).join(name), data);
    }
}

fuzz_target!(|data: &[u8]| {
    contain_decoder_panics();
    if let Ok(mut demuxer) = Demuxer::open(Box::new(Bytes::new(data.to_vec())), &Hint::default()) {
        let tracks = demuxer.tracks().to_vec();
        let mut sounds: Vec<(u32, SoundDecoder)> = tracks
            .iter()
            .filter(|t| t.is_sound() && t.decodable())
            .filter_map(|t| SoundDecoder::new(t).ok().map(|d| (t.id, d)))
            .collect();
        let mut pictures: Vec<(u32, PictureDecoder)> = tracks
            .iter()
            .filter(|t| t.is_picture() && t.decodable())
            .filter_map(|t| PictureDecoder::new(t).ok().map(|d| (t.id, d)))
            .collect();
        'rounds: for round in 0..2 {
            let mut read = 0;
            loop {
                let packet = match demuxer.next_packet() {
                    Ok(Some(packet)) => packet,
                    Ok(None) => break,
                    Err(Error::Panicked) => {
                        keep_panicking_input(data);
                        break 'rounds;
                    }
                    Err(_) => break,
                };
                read += 1;
                if read > MAX_PACKETS {
                    break;
                }
                for (id, decoder) in &mut sounds {
                    if *id != packet.track {
                        continue;
                    }
                    match decoder.decode(&packet) {
                        Ok(sound) => {
                            let channels = usize::from(sound.channels.max(1));
                            assert_eq!(sound.samples.len() % channels, 0);
                            assert!(sound.samples.iter().all(|s| s.is_finite()));
                        }
                        Err(Error::Panicked) => {
                            keep_panicking_input(data);
                            assert_eq!(decoder.decode(&packet).err(), Some(Error::Panicked));
                        }
                        Err(_) => {}
                    }
                }
                for (id, decoder) in &mut pictures {
                    if *id != packet.track {
                        continue;
                    }
                    match decoder.decode(&packet) {
                        Ok(decoded) => {
                            for p in decoded {
                                assert!(p.width <= MAX_VIDEO_SIZE && p.height <= MAX_VIDEO_SIZE);
                                assert_eq!(p.rgba.len(), p.width as usize * p.height as usize * 4);
                                assert!(p.rgba.chunks(4).all(|px| px[3] == 255));
                            }
                        }
                        Err(Error::Panicked) => {
                            keep_panicking_input(data);
                            assert_eq!(decoder.decode(&packet).err(), Some(Error::Panicked));
                        }
                        Err(_) => {}
                    }
                }
            }
            // Then back to the middle, and through again.
            if round == 0 {
                let Some(track) = tracks.first() else { break };
                match demuxer.seek(0.5, track.id) {
                    Ok(_) => {}
                    Err(Error::Panicked) => {
                        keep_panicking_input(data);
                        break;
                    }
                    Err(_) => break,
                }
                for (_, decoder) in &mut sounds {
                    decoder.reset();
                }
                for (_, decoder) in &mut pictures {
                    let _ = decoder.reset();
                }
            }
        }
    }
    if let Err(Error::Panicked) = decode_all(data, 1 << 24) {
        keep_panicking_input(data);
    }
});
