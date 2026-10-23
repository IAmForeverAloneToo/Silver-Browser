# Media fixtures

The clips the `media` crate's tests and the engine's media tests play
(M40, `docs/design/audio-video.md`). Their content is known:

- the pictures: 160 by 120, ten frames a second for two seconds, red
  (`#ff0000`) for the first second and green (`#00ff00`) for the second,
  a keyframe at each second, AV1 (`av01.0.04M.08`);
- the sound: Opus at 48 kHz in stereo, 96 kb/s, two seconds of a 440 Hz
  sine of amplitude 0.5 on the left channel and silence on the right;
  its encoder's pre-skip is 312 frames.

Both were encoded by the reference browser itself (Chrome 155, its
WebCodecs `VideoEncoder` and `AudioEncoder`), and muxed into

- `clip.webm`: AV1 and Opus in WebM, with cues;
- `clip-video.webm`: the AV1 alone;
- `clip-audio.webm`: the Opus alone;
- `clip.opus.ogg`: the Opus in Ogg;
- `clip.mp4`: AV1 (`av01`, `av1C`) and Opus (`Opus`, `dOps`) in MP4.

`tone.wav` (16-bit PCM) holds one second of the same sine at 48 kHz,
amplitude 16383 of 32767, the right channel silent; it was written
directly, not encoded by the reference.

The reference plays each of the first five and the WAV as made:
`duration` 2.1, 2.0, 2.02, 2.0065, 2.02 and 1 seconds, the pictures red
at 0.5 s and green at 1.5 s, the sound's left peak 0.5175 and its right
silent. There is no FLAC fixture: no FLAC encoder is at hand, and a
file written here was refused by the reference's demuxer and by
symphonia's alike.
