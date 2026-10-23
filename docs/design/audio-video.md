# Design note: audio, video and Web Audio

Roadmap M40. Written before the code, as the record of the decisions,
and corrected to the code when the milestone ends: where they
disagree, the code wins, and section 4 lists the places, marked
*(corrected)*. What ships is described in `docs/architecture.md`
"Current state", and the limits it adds live in `docs/security.md`.

A page here shows text, images, canvases, SVG and frames; it plays
nothing. `<video>` and `<audio>` are elements with reflected attributes
(`src`, `autoplay`, `loop`, `controls`, `poster`, `width`, `height`)
and no box of their own; `HTMLMediaElement` has no `play()`, no
`currentTime`, no events; there is no `AudioContext`; nothing in the
browser opens an audio device; a `video` is never a canvas's image
source. The network fetches a subresource whole, never by a range, and
Opaque Response Blocking reads a media request's state as never
"initial" or "subsequent", for "this version plays no media"
(`service/orb.rs`). A page whose content is a clip, a song or a sound,
or a game or an instrument that makes its sound with Web Audio, does
not work.

This milestone gives the engine media: `<video>` and `<audio>` that
fetch their resource by ranges, demux and decode it on threads of the
renderer, play it with its sound through the browser's own audio
output and its pictures painted at the media clock's time, and take the
standard's load, play, pause, seek and end with their events and
promises; basic controls drawn by the engine; the Web Audio API's
graph, rendered in quanta of 128 frames, live through the same output
and offline into a buffer; the Media Session API, and the keyboard's
media keys acting on it; and never DRM. Its acceptance is a page whose
`<video controls>` plays a clip with its sound, pauses, seeks and ends
as the reference browser's does, and Web Audio's graphs rendering the
reference's numbers.

The documents this note follows, at the versions current when it was
written (23 October 2026):

- the HTML Standard, "Media elements" (section 4.8.11): the
  `HTMLMediaElement`, `HTMLVideoElement` and `HTMLAudioElement`
  interfaces, the media element load algorithm, the resource selection
  algorithm, the resource fetch algorithm, the ready states and network
  states, playing, pausing, seeking, the ended playback, the official
  playback position, `TimeRanges`, `MediaError`, the `poster` frame,
  `canPlayType()`, the media element event summary; "User activation"
  (sticky and transient activation);
- Web Audio API 1.1 (W3C Candidate Recommendation Draft of 2026): the
  `BaseAudioContext`, `AudioContext` and `OfflineAudioContext`
  interfaces, `AudioBuffer`, `AudioNode` and `AudioParam` with its
  automation, the nodes, the rendering algorithm and its render
  quantum, channel up-mixing and down-mixing, `decodeAudioData()`;
- Media Session (W3C Working Draft): `MediaSession`, `MediaMetadata`,
  the actions and their handlers, the position state;
- Fetch: range requests for media (the `Range` header a media element's
  request carries), the `audio`, `video` and `track` destinations;
  Opaque Response Blocking's media request states;
- MIME Sniffing: the rules for audio and video;
- the codecs' and containers' own: Matroska and WebM, ISO BMFF (MP4),
  Ogg (RFC 3533) and its Opus (RFC 7845) and Vorbis mappings, RIFF
  WAVE, FLAC (RFC 9639), MPEG-1/2 Layer III, Opus (RFC 6716 and
  RFC 8251), Vorbis I, AV1 (the AV1 Bitstream and Decoding Process
  Specification) and its ISO BMFF and Matroska mappings.

What the standards leave open, or say but are worth checking, was put
to the reference browser (Chrome 155.0.8059.85, headless) before this
note was written, its pages served on `127.0.0.1` with range requests
answered (scratchpad `m40/pages`, `p1` to `p7`; the page posts what it
found to the probe's server). The clips the probes and the tests play
were made by the reference browser itself: its WebCodecs encoders made
a two-second AV1 picture (160 by 120, ten frames a second, red for the
first second and green for the second, a keyframe each second) and
Opus sound (48 kHz stereo, a 440 Hz sine of amplitude 0.5 on the left,
silence on the right), and a small muxer of ours put them in WebM, MP4
and Ogg; the reference plays each, its frames and samples as made.

- *What it plays.* `canPlayType()` answers `"probably"` for a container
  with codecs it decodes, `"maybe"` for a container alone
  (`video/webm`, `video/mp4`, `audio/ogg`, `audio/wav`, `audio/mp4`,
  `audio/webm`, `video/ogg`, `application/ogg`, `video/x-matroska`,
  `audio/x-wav`), `""` for what it does not (`video/quicktime`,
  `audio/x-flac`, `audio/wave`, `audio/opus`, `video/ogg;
  codecs="theora"`, an unknown codec, `text/plain`, the empty string);
  `audio/mpeg`, `audio/mp3`, `audio/flac` and `audio/aac` are
  `"probably"` with no codecs named. Being the branded build, it says
  `"probably"` to H.264 and AAC (`avc1`, `mp4a.40.2`) as well.
- *A new element.* `readyState` 0, `networkState` 0, `paused`,
  `volume` 1, `preload` `"metadata"` for a `<video>` and `"auto"` for
  `new Audio()`, `duration` `NaN`, `playbackRate` and
  `defaultPlaybackRate` 1, `preservesPitch` true, empty `TimeRanges`
  for `buffered`, `seekable` and `played`, `error` `null`. Its box:
  a `<video>` 300 by 150 inline, with or without `controls`; an
  `<audio>` with `controls` 300 by 54, without them `display: none`;
  a `<video>` whose metadata came, its natural size (160 by 120), its
  `object-fit` `contain`.
- *Loading.* With the media element events recorded with their ready
  and network states at dispatch: `loadstart` (0, 2), `progress`,
  `suspend` (network idle), then `durationchange`, `resize`,
  `loadedmetadata`, `loadeddata`, `canplay`, `canplaythrough`, each
  dispatched once the ready state is already 4: events are queued as
  tasks, the state moves on before they run. For sound alone,
  `durationchange` and `loadedmetadata` at 1, then `loadeddata` at 4.
  A clip's `duration` is its container's (2.1, 2.02, 2.0065 seconds as
  muxed); `buffered` and `seekable` the whole clip once fetched.
- *Seeking and playing.* Setting `currentTime` fires `seeking` (ready
  state 1), `timeupdate` and `seeked`; the frame then shown is the
  one of that time (red at 0.5 s, green at 1.5 s). `play()` resolves
  once playing; `play`, `playing`, then `timeupdate` about four times
  a second, and at the end `pause` before `ended`, `paused` true,
  `currentTime` the duration, `played` the range played.
- *The autoplay policy.* With no user activation, `play()` of a
  sounding element rejects with `NotAllowedError` "play() failed
  because the user didn't interact with the document first.
  https://goo.gl/xX8pDD", and `autoplay` does not start it; a `muted`
  one plays. An `AudioContext` starts `"suspended"` and its `resume()`
  stays pending until the user interacts. With autoplay allowed, the
  same `AudioContext` is `"running"`.
- *Errors.* A source that does not load (a `404`, a text file) is
  `MediaError` code 4 with the message "MEDIA_ELEMENT_ERROR: Format
  error", network state 3, ready state 0; `src=""` is code 4 "...:
  Empty src attribute". `play()` then rejects `NotSupportedError` "The
  element has no supported sources."; `play()` interrupted by `pause()`
  rejects `AbortError` "The play() request was interrupted by a call to
  pause(). https://goo.gl/LdLk22", by a new load "... by a new load
  request. https://goo.gl/LdLk22". With `<source>` children, one whose
  `type` it cannot play is passed over, one that fails to load fires
  `error` at the `<source>` and the next is tried; when none is left,
  network state 3 and `error` stays `null`, and `play()` waits.
  `volume = 2` throws `IndexSizeError` "Failed to set the 'volume'
  property on 'HTMLMediaElement': The volume provided (2) is outside
  the range [0, 1]."; `playbackRate = 20` throws `NotSupportedError`
  "... The provided playback rate (20) is not in the supported playback
  range."; a negative `currentTime` is taken (clamped).
  `TimeRanges.start(0)` of an empty range throws `IndexSizeError` "The
  index provided (0) is greater than or equal to the maximum bound
  (0).".
- *Web Audio's numbers.* An `OfflineAudioContext` at 48 kHz renders, of
  a 440 Hz `OscillatorNode`, `sin(2 pi 440 n / 48000)` to six places
  (`0.057564`, `0.114937`, ... `-0.500000` at frame 100); its square,
  sawtooth and triangle as band-limited wave tables (the square
  `0.950091` at frame 1, not 1); a gain automated by
  `setValueAtTime`, `linearRampToValueAtTime`,
  `exponentialRampToValueAtTime` and `setTargetAtTime` the standard's
  curves at each frame; every `BiquadFilterNode` type's impulse
  response (the low-pass at 1 kHz and Q 1: `0.004042`, `0.015660`,
  `0.029790`, ...); a `DelayNode` of 10 ms moving an impulse by 480
  frames exactly; `StereoPannerNode` at 0.5 on a mono 1 giving
  `0.382683` and `0.923880` (equal power); mono up-mixed to stereo as
  copies; an `AudioBufferSourceNode` at `playbackRate` 0.5 interpolating
  linearly, and looping between its loop points; a `WaveShaperNode`
  interpolating its curve; a `ConvolverNode` convolving exactly when
  not normalized, and scaling by the standard's normalization when it
  is; the equal-power `PannerNode` at x = 1 all right; a
  `DynamicsCompressorNode` on a constant 0.9 starting at 0 and settling
  toward 0.57 (its look-ahead delay and attack). The errors are the
  reference's words: "Failed to construct 'OfflineAudioContext': The
  number of frames provided (0) is less than the minimum bound (1).",
  "... The sampleRate provided (1000) is outside the range [3000,
  768000].", "... The number of channels provided (0) is outside the
  range [1, 32].", `connect()` across contexts "cannot connect to an
  AudioNode belonging to a different audio context."
  (`InvalidAccessError`), `start()` twice "cannot call start more than
  once." (`InvalidStateError`), `createDelay(200)` "The max delay time
  provided (200) is outside the range (0, 180).", an exponential ramp
  to 0 a `RangeError`, `fftSize` 100 "The value provided (100) is not a
  power of two." (`IndexSizeError`), a non-finite `AudioParam` value a
  `TypeError`; `decodeAudioData()` of bytes it cannot read rejects
  `EncodingError` "Unable to decode audio data", and of a 48 kHz WAV
  into a 44.1 kHz context gives 44,099 frames, resampled.
- *Media Session.* `navigator.mediaSession` has `playbackState`
  `"none"` and `metadata` `null`; `MediaMetadata` keeps `title`,
  `artist`, `album`, `artwork` (its `src` resolved) and an empty
  `chapterInfo`; `setActionHandler` takes `play`, `pause`,
  `seekbackward`, `seekforward`, `previoustrack`, `nexttrack`,
  `skipad`, `stop`, `seekto`, `togglemicrophone`, `togglecamera`,
  `hangup`, `previousslide`, `nextslide`, `enterpictureinpicture`, and
  throws `TypeError` "The provided value 'bogus' is not a valid enum
  value of type MediaSessionAction." for another; `setPositionState`
  refuses a position past the duration, a negative duration and a
  rate of zero with the reference's words.
- *What exists.* `HTMLVideoElement` has `requestVideoFrameCallback`,
  `getVideoPlaybackQuality`, `requestPictureInPicture`; media elements
  `captureStream`, `setSinkId` and `sinkId`, `mediaKeys` and
  `setMediaKeys`, `textTracks` and `addTextTrack`, `srcObject`,
  `remote`; no `audioTracks`, `videoTracks`, `fastSeek` or
  `getStartDate`. `AudioWorkletNode`, `ScriptProcessorNode`,
  `IIRFilterNode`, `MediaSource` and `MediaKeys` are interfaces.

## 1. Decisions

### What it plays (D36)

**The codecs of Chromium's open-source build.** Sound: Opus, Vorbis,
FLAC, MP3, and PCM (8, 16, 24 and 32-bit integer and 32-bit float,
little-endian). Pictures: AV1. Containers: WebM and Matroska, ISO BMFF
(MP4 and M4A), Ogg, WAV, FLAC's own and MP3's. H.264 and AAC are left
out, as Chromium leaves them out without `proprietary_codecs`: they are
patent-encumbered, and their decoders are not what an open project
ships unlicensed. VP8 and VP9 are left for a later item (G-list): no
decoder of them in Rust is mature enough to meet untrusted bytes, and
writing them is a milestone's work of its own. `canPlayType()` answers
as the reference does for what both play, and `""` for `avc1`, `mp4a`
and `audio/aac`, as Chromium's open-source build answers; a container
alone is `"maybe"`.

**Decoders are crates, read as the images' are.** `symphonia` 0.6
(MPL-2.0, safe Rust) demuxes every container named and decodes MP3,
Vorbis, FLAC and PCM; `opus-decoder` 0.1 (safe Rust, `forbid(unsafe)`)
decodes Opus; `rav1d` 1.1 (BSD-2-Clause, the Rust port of `dav1d`,
built without its assembly) decodes AV1. They are reached through a new
crate, `media`, which owns what is read: a container's tracks, its
packets with their times, the decoders' output as interleaved `f32`
samples and as RGBA pictures (YUV converted by the stream's own matrix,
range and chroma siting: BT.601, BT.709, BT.2020, limited or full), and
seeking. `rav1d`'s interface is `dav1d`'s C one: `media` says
`#![deny(unsafe_code)]` and allows it in its `av1` module alone, each
block argued (`dav1d_default_settings`, `dav1d_open`,
`dav1d_data_create` and its copy, `dav1d_send_data`,
`dav1d_get_picture`, the picture's planes read as slices of their
strides and heights, `dav1d_picture_unref`, `dav1d_close`), as `js`
does its bytecode's one module (M35.12); the
shell's test that holds every source file to the rule names it.

### Where media runs (D37)

**Decoding in the renderer, sound out of the browser.** A media
element's resource is demuxed and decoded in the renderer of its
document, on a thread of its own (the element's *pipeline*), sandboxed
as everything else there; `decodeAudioData()` runs on the renderer's
decode pool. Sound reaches the speakers through the browser process,
which alone may open an audio device: an *audio output* in the shell's
platform module, on Windows a WASAPI shared-mode stream of the default
device's mix format (32-bit float, its rate, in practice 48 kHz,
stereo or more), fed by a thread that mixes what each renderer sends.
A renderer has one *mixer*, a thread that renders each running
`AudioContext`'s graph and each playing element's sound at the
device's rate, mixes them, and sends the mix to the browser in blocks
(`ToBrowser::Audio`); the browser says what it played
(`ToRenderer::AudioPlayed`), by which the mixer keeps 30 ms ahead and
no more. On macOS and Linux there is no audio output yet (G-list): the
mixer's sink plays nothing and keeps time by the system's clock, so
pictures and `currentTime` move as with sound. In one process the
engine's `Context` takes an `AudioSink` the shell gives (the same
output) or a null sink that plays nothing at real time; the tests use a
sink that keeps what it is given and plays it at once, or at the
system's clock.

**The media clock is the sound's.** An element with sound keeps its
official playback position by the frames of its sound the output says
it played, its latency allowed for; without sound, or with a sink that
plays nothing, by the system's monotonic clock. Pictures are not
pushed: the pipeline decodes them ahead into a short queue, and each
frame the engine paints takes the picture whose time is the latest at
or before the media clock's, dropping those it passed. The engine
needs a frame while a video plays (`needs_frame`), and asks to be woken
at the next picture's time (`next_frame_at`).

### Fetching media

**By ranges, into a buffer the pipeline reads.** An element's request
has the destination `video` or `audio`, its mode `no-cors` or, with a
`crossorigin` attribute, `cors` with that credentials mode, and
`Range: bytes=0-` as the reference sends. A `206` with a
`Content-Range` makes the resource *seekable by ranges*: a seek to
bytes not yet fetched starts a new ranged request there; a `200`
streams the whole, seekable within what came. What comes is kept in
the element's *media buffer*, byte ranges in memory, within
`MAX_MEDIA_BUFFER` (64 MiB) an element, the ranges farthest behind the
playback position dropped first and fetched again if wanted. The
pipeline reads the buffer as a file (symphonia's `MediaSource`),
blocking until the bytes it wants come or the fetch fails. The engine's
`Network` streams a media target's body as it streams a document's
(today only documents stream); the browser's network service does the
same for a renderer's. The HTTP cache keeps no partial response, as
today.

**Opaque Response Blocking's media requests.** A `no-cors` media
request of another origin is "initial" at its first request and
"subsequent" for the ranges after it, as the standard says: its first
response must be sniffed as audio or video, or one of the ORB
safelist, and a later range answers only a request whose first was
admitted (`service/orb.rs`). A `cors` one is checked by CORS. An
element fed by another origin without CORS is *tainted*: a canvas that
draws it is tainted, and a `MediaElementAudioSourceNode` of it outputs
silence, as the standard says.

### The media element

**The standard's algorithms, in Rust, the element's state in the
engine.** `engine::media` keeps each media element's state: its
network and ready states, the current and official playback positions,
`paused`, `ended`, `seeking`, its pending play promises, its source
being tried, its error, its volume and muted, its rates, and the
pipeline it drives. The load algorithm (abort what was, `emptied`,
reset), the resource selection algorithm (the `src` attribute, else the
`<source>` children in order, a `type` it cannot play passed over,
`error` at a `<source>` that fails), the resource fetch algorithm, the
ready states (`HAVE_METADATA` once the tracks and the duration are
known, `HAVE_CURRENT_DATA` once the picture or sound of the current
position is decoded, `HAVE_FUTURE_DATA` and `HAVE_ENOUGH_DATA` by what
is decoded and buffered ahead against the rate the bytes come), play,
pause, internal play steps, seeking (to the keyframe before, decoded
forward to the exact time, `seeking`, `timeupdate`, `seeked`), ended
playback (`timeupdate`, `pause`, `ended`; with `loop`, a seek to the
start), `timeupdate` every 250 ms while playing and at the steps the
standard names, are the standard's, their events queued as tasks on
the element's media element event task source, so that a script sees
them with the states as the reference shows them. `play()`'s promises
are kept and settled as the standard says, with the reference's
messages. The JavaScript side is the bootstrap's `HTMLMediaElement`,
`HTMLVideoElement`, `HTMLAudioElement` (and `Audio`), `MediaError`,
`TimeRanges`, calling one native (`native.media`) and hearing one task
(`Task::Media`), as the canvas and IndexedDB do.

**Rates, volume, sound.** `playbackRate` is taken from 0.0625 to 16,
as the reference's range; another throws its error. Sound at another
rate is resampled, its pitch moving with it: `preservesPitch` is kept
and reflected but has no effect yet (G-list). `volume` and `muted`
scale the element's sound in the mixer. Several elements and contexts
of a renderer mix; several renderers mix in the browser.

**The autoplay policy.** A document gains *sticky activation* (the
HTML Standard's "has been activated") at its first activation, beside
the transient activation the frame tree keeps today. A sounding
element's `play()`, and `autoplay`, are allowed in a document with
sticky activation, or one whose top-level document has it and is of
its site; a muted element, or one with no sound, always; otherwise
`play()` rejects with the reference's `NotAllowedError`. An
`AudioContext` made without activation is `"suspended"`, and
`resume()` waits for an activation, at which every waiting context of
the document starts, as the reference does. Autoplay Policy Detection's
`navigator.getAutoplayPolicy()` is not given: the reference has none
(probe p7).

**Painting a video.** A `<video>` gets a replaced box
(`Replaced::Video`, beside `Canvas`): its natural size the video's
once known, else the poster's, else none (300 by 150); `object-fit` and
`object-position` placing its picture as an image's. Its picture is an
image painted as a canvas's is, its id the node's, its pixels the
current picture's, given at paint (`Engine::show_pixels`); before the
first picture, the poster, or nothing. A canvas's `drawImage()` and
`createImageBitmap()` take a video's current picture (the canvas
tainted as above); a video with no picture draws nothing.

**Controls drawn by the engine.** With `controls`, the element paints
the reference's basic controls over its box's bottom: a bar with a
play and pause button, the current time and the duration, a timeline
that seeks when clicked or dragged, a mute button, and on `<audio>` the
same in its 300 by 54 box; they answer the pointer and, when the
element has focus, the space and arrow keys. They are the engine's own
painting and hit testing, not a shadow tree (the engine has no
user-agent shadow trees for controls, M26); their look follows the
reference's at a glance, not to the pixel. Fullscreen and
picture-in-picture are not there (G-list).

### Web Audio

**The graph in a crate of its own, rendered on the mixer.** A new
crate, `audio`, `#![forbid(unsafe_code)]`, holds the rendering: the
graph of nodes with their inputs, outputs and channel rules, the
render quantum of 128 frames, up-mixing and down-mixing by the
standard's speaker and discrete rules, `AudioParam` automation (its
events, their curves, `a-rate` and `k-rate`, the computed value with
its inputs), and the nodes: `AudioDestinationNode`,
`AudioBufferSourceNode` (rate, detune, loop, start and stop with
offsets), `OscillatorNode` and `PeriodicWave` (band-limited wave
tables as the standard describes, the reference's numbers),
`ConstantSourceNode`, `GainNode`, `BiquadFilterNode` (the eight types,
the Audio EQ Cookbook's coefficients), `IIRFilterNode`, `DelayNode`
(and cycles through it), `StereoPannerNode`, `PannerNode` with the
equal-power model and its distance and cone rules, and `AudioListener`,
`ChannelSplitterNode`, `ChannelMergerNode`, `WaveShaperNode` (with 2x
and 4x oversampling), `DynamicsCompressorNode` (the reference's
algorithm as the standard describes it), `ConvolverNode` (partitioned
convolution, normalized as the standard says), `AnalyserNode` (its FFT,
Blackman window, smoothing and dB range), and
`MediaElementAudioSourceNode`, which takes an element's sound out of
the mixer and into the graph. `decodeAudioData()` decodes with `media`
and resamples to the context's rate with a windowed-sinc resampler.

**Two threads, as the standard has them.** The bootstrap's objects
are the control thread's: each call checks its arguments with the
reference's words and sends a message to the rendering thread (the
renderer's mixer for an `AudioContext`, a thread of its own for an
`OfflineAudioContext`'s `startRendering()`), which applies messages
between quanta. What the rendering thread tells back (a source's
`ended`, an `AnalyserNode`'s latest data, `currentTime`, a context's
state) comes as tasks. An `OfflineAudioContext` renders as fast as it
can and resolves with its `AudioBuffer`; `suspend(time)` stops it at
that quantum until `resume()`.

### Media Session and the media keys

`navigator.mediaSession`, `MediaMetadata` and the action handlers are
the document's; the browser keeps, for each tab, its *active session*:
the top-level document's, whose state is its `playbackState` or, when
`"none"`, whether one of the tab's elements plays. The keyboard's media
keys (play and pause, stop, next and previous track) reach the shell
from the window as keys, and act on the focused window's active tab:
the action's handler if the page set one, else the default action
(play or pause the element that played last, stop pausing it). The
operating system's media controls (Windows's System Media Transport
Controls, macOS's Now Playing) and media keys pressed while another
window has focus are not there (G-list).

### Never DRM

Encrypted Media Extensions are explicitly never here (the roadmap: DRM
is broken by design). `navigator.requestMediaKeySystemAccess()` exists
and rejects every key system with the reference's `NotSupportedError`
"Unsupported keySystem or supportedConfigurations.", as a browser
without a content decryption module answers (the reference grants Clear
Key and Widevine, and refuses another key system so; probe p7), so
that a page falls back;
`MediaKeys` and its kin are absent, and `setMediaKeys(null)` resolves.
Clear Key, which is no DRM, is left out with the rest.

### The processes' messages

Renderer to browser: `Audio { stream, frames }` (interleaved `f32` at
the output's rate and channels, at most `MAX_AUDIO_BLOCK` frames),
`AudioStream { stream, open }`, and `MediaSession { tab state }` for the
active session's state. Browser to renderer: `AudioFormat { rate,
channels }` at the hello, `AudioPlayed { stream, frames, latency }`,
and `MediaKey { key }`. A renderer may have one stream; its blocks are
bounded and its queue in the browser is bounded at 200 ms, past which
blocks are dropped. Fetches keep their messages: a media target streams
as a document does.

### DevTools

The network panel shows media requests with their ranges and `206`s;
the console says why a source failed. A media panel (the pipeline's
state, the decoders' figures) is not there (G-list).

### Bounds

`MAX_MEDIA_BUFFER` (64 MiB an element), `MAX_MEDIA_PIPELINES` (75 a
renderer, the reference's limit of players a renderer; past it a new
element's load fails with a format error until one ends),
`MAX_VIDEO_SIZE` (8,192 by 8,192 pictures; a larger one fails as a
decode error), `MAX_DECODED_PICTURES` (8 queued ahead an element),
`MAX_DECODED_SOUND` (1 second ahead), `MAX_AUDIO_CONTEXTS` (a renderer's
live contexts, 32), `MAX_AUDIO_NODES` (100,000 a context),
`MAX_AUDIO_BUFFER_BYTES` (an `AudioBuffer`, 512 MiB, and a context's
buffers together), `MAX_DECODE_AUDIO_BYTES` (`decodeAudioData()`'s
input and output, 512 MiB), `MAX_OFFLINE_FRAMES` (an
`OfflineAudioContext`'s length, the buffer bound), `MAX_DELAY_SECONDS`
(180, the standard's), `MAX_CONVOLVER_FRAMES` (an impulse response,
10 seconds at 48 kHz), `MAX_AUDIO_BLOCK` and the browser's queue, and
the render thread's budget: a quantum not rendered in time is late, not
waited for, and a renderer whose mixer falls behind loses blocks rather
than holding the output.

### Fuzzing

`media_demux` (any bytes as each container, its packets decoded by the
track's decoder, within the bounds and never panicking), `av1_decode`
(any OBUs to `rav1d` through `media`'s module, every picture converted),
`audio_graph` (any sequence of node creations, connections,
disconnections, automation events and renders against the standard's
invariants: no panic, finite output, bounds kept, a graph's output the
same rendered whole or by quanta); `ipc_decode`'s seeds gain the new
messages.

### The acceptance (M40.6)

A page with `<video controls>` of the AV1 and Opus clip in WebM plays
with its sound reaching the output (the 440 Hz on the left), its
pictures the right colour at their times, its events and states as the
reference's (`loadstart` through `ended`), pauses and seeks through its
controls by the pointer, and with the media key; the same of MP4 and of
the sound alone in Ogg, WebM and WAV. An `AudioContext` page's sound
reaches the output once the user interacts. Web Audio's offline graphs
give the reference's numbers (probe p5) to the standard's tolerance.
The web-platform tests' `webaudio` and
`html/semantics/embedded-content/media-elements` directories, at the
subset's commit, the tests the engine can run, with expectations. A
bench of a video's frame (decode, convert and paint) and of a graph's
quantum.

## 2. Not in this milestone, said rather than hidden

- *VP8, VP9, H.264, AAC, Theora* (above); WPT's clips are mostly these,
  so many of its media tests are expected failures.
- *Media Source Extensions* (`MediaSource`, `SourceBuffer`) and
  *WebCodecs*: each a milestone's.
- *`AudioWorklet`* and *`ScriptProcessorNode`*: script on the
  rendering thread, a realm of its own there.
- *`MediaStream`* and what takes or makes one: `srcObject`,
  `captureStream()`, `MediaStreamAudioSourceNode` and its destination
  (M43, WebRTC, holds streams).
- *The HRTF panning model* (its impulse responses are data the
  reference ships), `setSinkId()` and choosing a device.
- *Text tracks*: `<track>`, WebVTT, `textTracks` beyond an empty list.
- *Fullscreen, picture-in-picture, `requestVideoFrameCallback()`,
  `getVideoPlaybackQuality()`*, `audioTracks` and `videoTracks`.
- *`preservesPitch`*'s effect (above).
- *The operating system's media controls* and global media keys.
- *Audio output on macOS and Linux* (CoreAudio, PulseAudio or ALSA).
- *A media panel* in DevTools.
- *Encrypted Media Extensions*: never.

## 3. Order of work

1. *The `media` crate*: containers and decoders through the crates, the
   AV1 module, pictures to RGBA, seeking, the resampler; the fixtures;
   `media_demux` and `av1_decode`.
2. *The `audio` crate*: the graph, the quantum, mixing rules,
   `AudioParam` automation, the nodes; offline rendering against probe
   p5's numbers; `audio_graph`.
3. *Web Audio in the engine*: the bootstrap's interfaces, the control
   and rendering threads, `OfflineAudioContext`, `decodeAudioData()`,
   the reference's errors; sticky activation and `AudioContext`'s
   policy.
4. *The audio output*: the mixer, the sink in one process, the
   browser's output and WASAPI in the shell's platform module, the
   messages; `AudioContext` heard.
5. *Fetching media*: range requests, streaming of media targets in the
   engine and the browser, the media buffer, ORB's media states.
6. *The media element*: the algorithms, states, events and promises,
   the pipeline, sound through the mixer, the autoplay policy,
   `canPlayType()`, `MediaElementAudioSourceNode`.
7. *Video on screen*: the replaced box, pictures at the media clock,
   the poster, `drawImage()` of a video, the controls.
8. *Media Session, the media keys, never DRM.*
9. *The acceptance*, WPT's subsets and the benches.
10. *T4.3 and fuzzing.*
11. *Documents.*

## 4. Corrected to the code

What the work found that this note did not say, or said otherwise, by
step:

- **The media crate (step 1).** symphonia's readers needed help, each
  worked round in `media`: its MP4 reader reads `av01` but not `av1C`
  (the probe's bytes are searched for it), makes `dOps` an OpusHead
  still big-endian (a version-0 head is read so), seeks backwards in
  Matroska only by cues it met while opening (a seek that cannot go
  back reopens the stream), and trims Ogg Opus's end but not its
  pre-skip (trimmed here). Its packets carry no keyframe flag: an AV1
  unit's first frame header is read, and a seek backs off a second,
  then doubling, until it starts at one. opus-decoder overflows a shift
  in a checked build on a stream the reference encoded: overflow checks
  are off for it in the dev and fuzz profiles. There is no FLAC
  fixture: a FLAC written here was refused by the reference's demuxer
  and by symphonia's alike. The `av1` module has 12 `unsafe` blocks.
- **rav1d (steps 2, 3 and 10).** rav1d 1.1.0 panics in its error cleanup on
  a corrupted keyframe when single-threaded, and its threaded path
  deadlocks now and then on a malformed stream. The upstream fix (one
  line, not released) is in `vendor/rav1d`, the cached 1.1.0 patched in
  through `[patch.crates-io]` with its assembly, scripts and project
  files left out (`vendor/README.md`); two leaks of its C interface
  that CI's LeakSanitizer found, and a subtraction its fuzzing found to
  underflow where dav1d's unsigned C wraps (step 10), are mended there
  too, each marked "Silver:". The decoder runs on one thread with one frame in flight,
  starts over after a unit it refuses, and takes no unit until one
  brings a sequence header; a panic located in rav1d aborts the process
  (`media::abort_on_decoder_panic`), since unwinding from it is unsound.
- **The audio crate (step 2).** The reference's oscillator is
  Chromium's PeriodicWave, matched: a 4,096-frame table for each third
  of an octave, two read and mixed, scaled by the full-band table's
  peak. The convolver's normalization calibrates to -58 dB. A delay's
  frames are worked out in single precision. The compressor follows the
  standard's outline, not the reference's kernel, so its numbers differ
  from the reference's (its shape is tested). The panner's model is the
  equal-power one only. The bound on a context's nodes is the crate's
  `MAX_NODES`; an offline context's length is bounded by
  `MAX_AUDIO_BUFFER_BYTES`, with no bound of its own.
- **Web Audio in the engine (steps 3 and 9).** The control thread is
  the bootstrap, through one native, `native.audio` (JSON and samples);
  what the threads tell comes as `WorkerMessage::Audio`, then
  `Task::Audio`. Sticky activation is kept for the tab, on its frame
  tree, not the document or its site. A realtime context is
  `"suspended"` when made and turns `"running"` in a task with its
  `statechange`, as the standard's control message (step 9; step 3 had
  it running at once). `decodeAudioData()` copies the page's bytes and
  leaves its buffer whole. Web Audio's messages carry their samples
  uncounted against the inbox's bound.
- **The audio output (step 4).** One mixer for each process, whose sink
  the process sets: a renderer's sends its blocks to the browser, one
  process plays through the shell's output, and the default is a sink
  that plays at the system's clock. The output asks WASAPI for 48 kHz
  stereo floats and lets the system convert (`AUTOCONVERTPCM`), not the
  device's own mix format, so a device of more channels plays stereo.
  The messages are `ToBrowser::Audio` (at most `MAX_AUDIO_BLOCK_FRAMES`,
  8,192 frames; an empty block asks what was played) and
  `ToRenderer::AudioPlayed`; there is no stream, format, session or key
  message. The browser's queue for a renderer is a second
  (`MAX_AUDIO_QUEUE_FRAMES`); a block past the bound kills the renderer.
  A document with an open `AudioContext`, or a playing element, is not
  kept by the back-forward cache.
- **Fetching media (step 5).** The requests are bounded ranges of
  `MEDIA_RANGE` (16 MiB, within net's body bound), the next asked for as
  one ends, not `Range: bytes=0-`; a ranged request asks for no coding.
  A streamed head is checked and typed (CORS, the resource policy, the
  tainting) before it is handed over, as a whole response's is. ORB's
  media states admit an origin's URL for its renderer after the initial
  response's first 1,024 bytes sniff as media (at most 256 URLs a
  renderer).
- **The media element (steps 6 and 9).** The module is
  `engine::playback`. A `src` set during a run is loaded after it, and
  a script's changes are heard after the script: the resource
  selection algorithm runs whole then, not awaiting a stable state, so
  `networkState` and the events' order differ from the standard's in
  some tests (G-list). The pipeline is ready at its first picture and a
  quarter second of sound; the clock is the frames the mixer took, the
  system's without sound or once the sound ran out. `muted` follows its
  attribute until a script or the controls set it (step 9). A document
  whose scripts do not run hears its pipelines itself. A `<meta>`
  policy reaches the media fetches a script starts while the document
  is parsed (step 9, found by WPT's media-src tests: a video loaded
  under `media-src 'none'`).
- **Video on screen (step 7).** The box is `Replaced::Video` (boxed, as
  an image's header is); `audio:not([controls])` is `display: none` and
  a video's `object-fit` is `contain` in the user agent's sheet. The
  controls bar is 32 px; their geometry is one function that paint and
  the pointer share.
- **Media Session (step 8).** The session is the document's: no active
  session in the browser and no key message, since the media keys
  already reach the focused tab's document as keys, whose engine acts
  on them.
- **The acceptance (step 9).** The web-platform tests' runner lets
  sound start without the user's activation, as the reference is run
  for them. Most media element tests play H.264 (`common/media.js` asks
  for VP9 in WebM first, then MP4) and are expected failures by D36.
  The bench's figures are in `docs/perf-budgets.md`.
