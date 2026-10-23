//! Media (M40) from arbitrary bytes: each container's reader, its tracks'
//! packets decoded by their decoders, a seek, and `decodeAudioData()`'s
//! whole decode. Nothing may panic, a decoder's panics included, and what
//! decodes must keep its bounds: pictures at most `MAX_VIDEO_SIZE` a
//! side, their pixels exactly their size, opaque; sound finite, a whole
//! number of frames.

#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;
use media::{
    Bytes, Demuxer, Error, Hint, MAX_VIDEO_SIZE, PictureDecoder, SoundDecoder, decode_all,
};

/// The packets read of one input, at most.
const MAX_PACKETS: usize = 2_000;

fuzz_target!(|data: &[u8]| {
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
        for round in 0..2 {
            let mut read = 0;
            loop {
                let packet = match demuxer.next_packet() {
                    Ok(Some(packet)) => packet,
                    Ok(None) => break,
                    Err(e) => {
                        assert_ne!(e, Error::Panicked, "a reader panicked");
                        break;
                    }
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
                        Err(e) => assert_ne!(e, Error::Panicked, "a sound decoder panicked"),
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
                        Err(e) => assert_ne!(e, Error::Panicked, "a picture decoder panicked"),
                    }
                }
            }
            // Then back to the middle, and through again.
            if round == 0 {
                let Some(track) = tracks.first() else { break };
                if let Err(e) = demuxer.seek(0.5, track.id) {
                    assert_ne!(e, Error::Panicked, "a seek panicked");
                    break;
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
    if let Err(e) = decode_all(data, 1 << 24) {
        assert_ne!(e, Error::Panicked, "decodeAudioData's decode panicked");
    }
});
