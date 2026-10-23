//! The crate against the reference browser's clips (`fixtures/`).

use crate::*;

const CLIP_WEBM: &[u8] = include_bytes!("../fixtures/clip.webm");
const CLIP_VIDEO: &[u8] = include_bytes!("../fixtures/clip-video.webm");
const CLIP_AUDIO: &[u8] = include_bytes!("../fixtures/clip-audio.webm");
const CLIP_OGG: &[u8] = include_bytes!("../fixtures/clip.opus.ogg");
const CLIP_MP4: &[u8] = include_bytes!("../fixtures/clip.mp4");
const TONE_WAV: &[u8] = include_bytes!("../fixtures/tone.wav");

fn open(bytes: &[u8]) -> Demuxer {
    Demuxer::open(Box::new(Bytes::new(bytes.to_vec())), &Hint::default()).expect("it opens")
}

/// Every picture of `bytes`'s picture track.
fn pictures(bytes: &[u8]) -> Vec<Picture> {
    let mut demuxer = open(bytes);
    let track = demuxer.first_picture().expect("a picture track").clone();
    let mut decoder = PictureDecoder::new(&track).unwrap();
    let mut out = Vec::new();
    while let Some(packet) = demuxer.next_packet().unwrap() {
        if packet.track == track.id {
            out.extend(decoder.decode(&packet).unwrap());
        }
    }
    out.extend(decoder.drain().unwrap());
    out
}

fn near(a: [u8; 4], b: [u8; 3]) -> bool {
    a[..3].iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 3) && a[3] == 255
}

#[test]
fn the_clips_tracks_and_durations_are_read_from_each_container() {
    for (bytes, sound, picture, duration) in [
        (CLIP_WEBM, true, true, 2.1),
        (CLIP_VIDEO, false, true, 2.0),
        (CLIP_AUDIO, true, false, 2.02),
        (CLIP_OGG, true, false, 2.0065),
        (CLIP_MP4, true, true, 2.02),
        (TONE_WAV, true, false, 1.0),
    ] {
        let demuxer = open(bytes);
        assert_eq!(demuxer.first_sound().is_some(), sound, "{demuxer:?}");
        assert_eq!(demuxer.first_picture().is_some(), picture, "{demuxer:?}");
        let got = demuxer.duration().expect("a duration");
        assert!(
            (got - duration).abs() < 0.03,
            "{got} for {duration}: {demuxer:?}"
        );
        if let Some(track) = demuxer.first_picture() {
            assert!(
                matches!(
                    track.kind,
                    TrackKind::Picture {
                        codec: PictureCodec::Av1,
                        width: 160,
                        height: 120,
                        ..
                    }
                ),
                "{track:?}"
            );
        }
        if let Some(track) = demuxer.first_sound() {
            assert!(
                matches!(
                    track.kind,
                    TrackKind::Sound {
                        channels: 2,
                        rate: 48_000,
                        ..
                    }
                ),
                "{track:?}"
            );
        }
    }
}

#[test]
fn av1_pictures_are_the_colours_of_their_times_as_the_reference_shows_them() {
    for bytes in [CLIP_WEBM, CLIP_VIDEO, CLIP_MP4] {
        let pictures = pictures(bytes);
        assert_eq!(pictures.len(), 20);
        for (i, p) in pictures.iter().enumerate() {
            assert!((p.time - i as f64 / 10.0).abs() < 1e-6, "{p:?}");
            assert_eq!((p.width, p.height), (160, 120));
            // The reference reads red as 255, 1, 0 and green as 0, 255, 1.
            let want = if i < 10 { [255, 1, 0] } else { [0, 255, 1] };
            assert!(near(p.pixel(80, 60), want), "{i}: {:?}", p.pixel(80, 60));
            assert!(near(p.pixel(0, 0), want) && near(p.pixel(159, 119), want));
        }
    }
}

#[test]
fn a_seek_lands_on_the_keyframe_before_and_decodes_to_its_time() {
    let mut demuxer = open(CLIP_VIDEO);
    let track = demuxer.first_picture().unwrap().clone();
    let at = demuxer.seek(1.5, track.id).unwrap();
    assert!(at <= 1.5 + 1e-9, "{at}");
    let mut decoder = PictureDecoder::new(&track).unwrap();
    let mut shown = None;
    while let Some(packet) = demuxer.next_packet().unwrap() {
        for p in decoder.decode(&packet).unwrap() {
            if p.time <= 1.5 + 1e-9 {
                shown = Some(p);
            }
        }
    }
    let shown = shown.expect("a picture at or before 1.5 s");
    assert!((shown.time - 1.5).abs() < 1e-6, "{shown:?}");
    assert!(near(shown.pixel(80, 60), [0, 255, 1]));
}

/// The left channel's peak and the right's, and the left's sample 100.
fn tone(samples: &[f32]) -> (f32, f32, f32) {
    let left = samples.iter().step_by(2).fold(0f32, |m, s| m.max(s.abs()));
    let right = samples
        .iter()
        .skip(1)
        .step_by(2)
        .fold(0f32, |m, s| m.max(s.abs()));
    (left, right, samples[200])
}

#[test]
fn opus_decodes_to_the_reference_browsers_samples_its_pre_skip_trimmed() {
    // The reference: 2 channels at 48 kHz, 96,000 frames from the Ogg
    // (its granule positions trim the end), left peak 0.5175, right
    // silent, left[100] -0.24600.
    let (rate, channels, samples) = decode_all(CLIP_OGG, 1 << 24).unwrap();
    assert_eq!((rate, channels), (48_000, 2));
    assert_eq!(samples.len() / 2, 96_000);
    let (left, right, at100) = tone(&samples);
    assert!((left - 0.5175).abs() < 0.002, "{left}");
    assert!(right < 1e-3, "{right}");
    assert!((at100 + 0.246).abs() < 0.002, "{at100}");
    // The WebM has no end trim: the reference gives 96,648 frames.
    let (_, _, webm) = decode_all(CLIP_AUDIO, 1 << 24).unwrap();
    assert_eq!(webm.len() / 2, 96_648);
    assert_eq!(&webm[..2000], &samples[..2000]);
    let (_, _, mp4) = decode_all(CLIP_MP4, 1 << 24).unwrap();
    assert_eq!(&mp4[..2000], &samples[..2000]);
}

#[test]
fn pcm_decodes_to_the_reference_browsers_samples() {
    let (rate, channels, wav) = decode_all(TONE_WAV, 1 << 24).unwrap();
    assert_eq!((rate, channels, wav.len()), (48_000, 2, 96_000));
    let (left, right, at100) = tone(&wav);
    assert!(
        (left - 0.49997).abs() < 1e-4 && right == 0.0,
        "{left} {right}"
    );
    // The reference: -0.24997.
    assert!((at100 + 0.24997).abs() < 1e-4, "{at100}");
}

#[test]
fn what_is_not_media_is_an_error_not_a_panic() {
    for bytes in [
        &b""[..],
        b"not media at all",
        &[0u8; 4096],
        &CLIP_WEBM[..200],
        &TONE_WAV[..30],
    ] {
        let result = Demuxer::open(Box::new(Bytes::new(bytes.to_vec())), &Hint::default());
        if let Ok(mut demuxer) = result {
            while let Ok(Some(_)) = demuxer.next_packet() {}
        }
        assert!(decode_all(bytes, 1 << 20).is_err());
    }
    // A decoded bound is kept.
    assert_eq!(
        decode_all(TONE_WAV, 1000),
        Err(Error::TooLarge("the decoded sound".to_owned()))
    );
}

#[test]
fn damaged_av1_packets_fail_or_decode_but_never_panic() {
    let mut demuxer = open(CLIP_VIDEO);
    let track = demuxer.first_picture().unwrap().clone();
    let mut decoder = PictureDecoder::new(&track).unwrap();
    let mut n = 0u32;
    while let Some(mut packet) = demuxer.next_packet().unwrap() {
        n += 1;
        for (i, b) in packet.data.iter_mut().enumerate() {
            if (i as u32 + n).is_multiple_of(7) {
                *b ^= 0x5A;
            }
        }
        let result = decoder.decode(&packet);
        assert_ne!(result.as_ref().err(), Some(&Error::Panicked));
    }
}

#[test]
fn can_play_type_answers_as_the_reference_for_what_both_play() {
    use CanPlay::*;
    for (text, want) in [
        ("video/webm", Maybe),
        ("video/webm; codecs=\"av01.0.04M.08, opus\"", Probably),
        ("video/webm; codecs=\"vp9, opus\"", No),
        ("video/webm; codecs=\"vp8, vorbis\"", No),
        ("video/mp4", Maybe),
        ("video/mp4; codecs=\"av01.0.04M.08, opus\"", Probably),
        ("video/mp4; codecs=\"avc1.42E01E, mp4a.40.2\"", No),
        ("video/mp4; codecs=\"flac\"", Probably),
        ("audio/ogg", Maybe),
        ("audio/ogg; codecs=\"opus\"", Probably),
        ("audio/ogg; codecs=\"vorbis\"", Probably),
        ("audio/ogg; codecs=\"flac\"", Probably),
        ("audio/mpeg", Probably),
        ("audio/mp3", Probably),
        ("audio/wav", Maybe),
        ("audio/wav; codecs=\"1\"", Probably),
        ("audio/wave", No),
        ("audio/x-wav", Maybe),
        ("audio/flac", Probably),
        ("audio/x-flac", No),
        ("audio/aac", No),
        ("audio/mp4", Maybe),
        ("audio/webm", Maybe),
        ("audio/webm; codecs=\"opus\"", Probably),
        ("video/ogg", Maybe),
        ("video/ogg; codecs=\"theora\"", No),
        ("application/ogg", Maybe),
        ("video/x-matroska", Maybe),
        ("audio/x-matroska", Maybe),
        ("video/quicktime", No),
        ("text/plain", No),
        ("", No),
        ("video/webm; codecs=\"bogus\"", No),
        ("audio/opus", No),
        ("audio/mpeg; codecs=\"mp3\"", Probably),
        ("VIDEO/WEBM; CODECS=\"OPUS\"", Probably),
    ] {
        assert_eq!(can_play_type(text), want, "{text}");
    }
}

#[test]
fn resampling_keeps_a_tone_and_gives_the_frames_the_ratio_says() {
    let sine: Vec<f32> = (0..48_000)
        .flat_map(|i| {
            let s = 0.5 * (2.0 * std::f64::consts::PI * 440.0 * f64::from(i) / 48_000.0).sin();
            [s as f32, 0.0]
        })
        .collect();
    let out = resample(&sine, 2, 48_000, 44_100);
    assert_eq!(out.len(), 44_100 * 2);
    for i in (1_000..43_000).step_by(997) {
        let want = 0.5 * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / 44_100.0).sin();
        assert!(
            (f64::from(out[i * 2]) - want).abs() < 0.002,
            "{i}: {} {want}",
            out[i * 2]
        );
        assert!(out[i * 2 + 1].abs() < 1e-6);
    }
    // Fed in pieces, the same.
    let mut r = Resampler::new(48_000.0, 44_100.0, 2);
    let mut pieces = Vec::new();
    for chunk in sine.chunks(2 * 333) {
        pieces.extend(r.process(chunk));
    }
    pieces.extend(r.flush());
    assert_eq!(&pieces[..out.len()], &out[..]);
}
