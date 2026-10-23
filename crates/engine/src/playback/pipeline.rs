//! A media element's pipeline (M40 step 6, D37): a thread that reads the
//! element's resource as its fetch lets it, demuxes it, and decodes its
//! first sound and picture tracks ahead of the playback position into
//! bounded queues: the sound, at the output's rate and channels, to the
//! process's mixer, whose frames played are the media clock; the
//! pictures, the latest at or before the clock shown. Without sound the
//! clock is the system's. It tells the element what it learns and when
//! (`Report`), and does what the element asks (play, pause, a seek, a
//! rate).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use media::{Demuxer, Hint, Picture, PictureDecoder, SoundDecoder};

use crate::audio_output::{OUTPUT_CHANNELS, OUTPUT_RATE};
use crate::loader::media::{MediaError, MediaResource};
use crate::webaudio::mixer;

/// The most pictures decoded ahead of the one shown.
pub(crate) const MAX_DECODED_PICTURES: usize = 8;
/// The most sound decoded ahead, in output frames: a second.
pub(crate) const MAX_DECODED_SOUND: usize = OUTPUT_RATE as usize;
/// The sound decoded before the element is told it can play: a quarter
/// of a second.
const READY_SOUND: usize = OUTPUT_RATE as usize / 4;
/// How often a playing element's time is told.
const TIME_UPDATE: Duration = Duration::from_millis(250);
/// How long the thread waits between looks when it has nothing to do.
const LOOK: Duration = Duration::from_millis(5);
/// How much of a resource still coming the pipeline waits for before it
/// reads its head.
const HEAD: u64 = 1 << 20;

/// What a pipeline knows of its resource once its head is read.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Meta {
    /// Its duration in seconds; infinite for a stream of no end.
    pub(crate) duration: f64,
    /// Its picture's size, for a video.
    pub(crate) size: Option<(u32, u32)>,
    pub(crate) sound: bool,
}

/// Why a pipeline failed: the media error's code and message.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Failure {
    pub(crate) code: u16,
    pub(crate) message: String,
}

/// What a pipeline tells its element.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Report {
    /// The fetch stopped for now: all came, or enough for a while.
    Fetched,
    /// The head of a resource of sound alone is read.
    Meta(Meta),
    /// The first picture, or sound, is decoded (and, for a video, the
    /// head read with it).
    Ready(Meta),
    /// Seek `id` is done, at that time.
    Seeked(u64, f64),
    /// The time, while playing.
    Time(f64),
    /// Playback reached the end, at that time.
    Ended(f64),
    Failed(Failure),
}

fn format_error() -> Failure {
    Failure {
        code: 4,
        message: "MEDIA_ELEMENT_ERROR: Format error".to_owned(),
    }
}

/// An element's sound as the mixer takes it: frames of the output's,
/// interleaved, queued; whether it plays; its gain (volume, muted); and
/// the frames taken since the clock was last set.
///
/// A `MediaElementAudioSourceNode` takes it into its graph (`feed`)
/// rather than the output; a resource of another origin without CORS
/// (`tainted`) gives it silence there.
#[derive(Default)]
pub(crate) struct ElementSound {
    pub(crate) queue: Mutex<VecDeque<f32>>,
    pub(crate) playing: AtomicBool,
    pub(crate) gain: AtomicU32,
    pub(crate) taken: AtomicU64,
    pub(crate) feed: Mutex<Option<audio::nodes::external::External>>,
    pub(crate) tainted: AtomicBool,
}

impl ElementSound {
    pub(crate) fn set_gain(&self, gain: f32) {
        self.gain.store(gain.to_bits(), Ordering::Relaxed);
    }

    pub(crate) fn gain(&self) -> f32 {
        f32::from_bits(self.gain.load(Ordering::Relaxed))
    }

    /// Takes up to `into.len()` samples, adding them at its gain to
    /// `into`, or giving them to the graph that took the sound; the
    /// frames it took.
    pub(crate) fn take_into(&self, into: &mut [f32]) -> usize {
        let gain = self.gain();
        let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        let n = queue.len().min(into.len()) / OUTPUT_CHANNELS * OUTPUT_CHANNELS;
        let feed = self
            .feed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        match feed {
            Some(external) => {
                let silent = self.tainted.load(Ordering::Relaxed);
                let samples: Vec<f32> = queue
                    .drain(..n)
                    .map(|s| if silent { 0.0 } else { s * gain })
                    .collect();
                external
                    .feed
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(&samples, OUTPUT_CHANNELS);
            }
            None => {
                for (o, s) in into.iter_mut().zip(queue.drain(..n)) {
                    *o += s * gain;
                }
            }
        }
        let frames = n / OUTPUT_CHANNELS;
        self.taken.fetch_add(frames as u64, Ordering::Relaxed);
        frames
    }
}

/// What the element asks and the pipeline keeps, under one lock.
struct State {
    playing: bool,
    rate: f64,
    seek: Option<(u64, f64)>,
    stopped: bool,
    meta: Option<Meta>,
    /// The pictures decoded, the first the one shown.
    pictures: VecDeque<Picture>,
    /// The clock: the media time it was last set at, and when (without
    /// sound, the system's time then).
    base: f64,
    since: Option<Instant>,
    /// Whether the demuxer has given its last packet.
    drained: bool,
    ended: bool,
    /// The sound ran out before the end: the clock is the system's to the
    /// end (a video's last pictures).
    dry: bool,
    /// Whether it was ready to play: until then, nothing plays and the
    /// clock stands, whether asked to play or not.
    ready: bool,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    sound: Arc<ElementSound>,
}

fn lock(shared: &Shared) -> MutexGuard<'_, State> {
    shared.state.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    /// The playback position the clock says.
    fn position(&self, state: &State) -> f64 {
        let has_sound = state.meta.as_ref().is_some_and(|m| m.sound) && !state.dry;
        let at = if has_sound {
            let taken = self.sound.taken.load(Ordering::Relaxed);
            state.base + taken as f64 / f64::from(OUTPUT_RATE) * state.rate
        } else {
            match state.since {
                Some(since) if state.playing && state.ready => {
                    state.base + since.elapsed().as_secs_f64() * state.rate
                }
                _ => state.base,
            }
        };
        match &state.meta {
            Some(meta) if meta.duration.is_finite() => at.min(meta.duration),
            _ => at,
        }
    }

    /// Sets the clock at `time`, and when.
    fn set_clock(&self, state: &mut State, time: f64) {
        state.base = time;
        state.since = Some(Instant::now());
        self.sound.taken.store(0, Ordering::Relaxed);
    }
}

/// An element's pipeline; it stops when dropped.
pub(crate) struct Pipeline {
    shared: Arc<Shared>,
    _mixing: mixer::Running,
}

impl Pipeline {
    /// Starts reading `resource`, which `hint` says what it may be,
    /// telling `tell` what it learns.
    pub(crate) fn start(
        resource: MediaResource,
        hint: Hint,
        tell: Box<dyn Fn(Report) + Send>,
        sound: Arc<ElementSound>,
    ) -> Pipeline {
        sound
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        sound.taken.store(0, Ordering::Relaxed);
        sound.tainted.store(false, Ordering::Relaxed);
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                playing: false,
                rate: 1.0,
                seek: None,
                stopped: false,
                meta: None,
                pictures: VecDeque::new(),
                base: 0.0,
                since: None,
                drained: false,
                ended: false,
                dry: false,
                ready: false,
            }),
            changed: Condvar::new(),
            sound: sound.clone(),
        });
        let mixing = mixer::add_element(&sound);
        let running = shared.clone();
        let _ = std::thread::Builder::new()
            .name("media pipeline".to_owned())
            .spawn(move || run(&running, &resource, &hint, &*tell));
        Pipeline {
            shared,
            _mixing: mixing,
        }
    }

    /// Plays or pauses.
    pub(crate) fn set_playing(&self, playing: bool) {
        let mut state = lock(&self.shared);
        if state.playing == playing {
            return;
        }
        let now = self.shared.position(&state);
        state.playing = playing;
        self.shared.set_clock(&mut state, now);
        if playing {
            state.ended = false;
        }
        self.shared
            .sound
            .playing
            .store(playing && state.ready, Ordering::Relaxed);
        drop(state);
        self.shared.changed.notify_all();
    }

    /// Seeks to `time`; seek `id` is told done.
    pub(crate) fn seek(&self, id: u64, time: f64) {
        lock(&self.shared).seek = Some((id, time));
        self.shared.changed.notify_all();
    }

    /// The playback rate from now: a seek to where it is, unheard.
    pub(crate) fn set_rate(&self, rate: f64) {
        let mut state = lock(&self.shared);
        if state.rate == rate {
            return;
        }
        let now = self.shared.position(&state);
        state.rate = rate;
        state.seek = Some((0, now));
        drop(state);
        self.shared.changed.notify_all();
    }

    /// The gain its sound is mixed at: its volume, or none muted.
    pub(crate) fn set_gain(&self, gain: f32) {
        self.shared.sound.set_gain(gain);
    }

    /// The current playback position.
    pub(crate) fn position(&self) -> f64 {
        let state = lock(&self.shared);
        self.shared.position(&state)
    }

    /// The picture shown at the current position, if one is decoded.
    #[allow(dead_code)]
    pub(crate) fn picture(&self) -> Option<Picture> {
        let mut state = lock(&self.shared);
        let now = self.shared.position(&state);
        while state.pictures.len() > 1 && state.pictures[1].time <= now + 1e-6 {
            state.pictures.pop_front();
        }
        let picture = state.pictures.front().cloned();
        drop(state);
        self.shared.changed.notify_all();
        picture
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        lock(&self.shared).stopped = true;
        self.shared.sound.playing.store(false, Ordering::Relaxed);
        self.shared.changed.notify_all();
    }
}

/// The decoders of a resource's first sound and picture tracks, and the
/// sound's way to the output's rate.
struct Decoders {
    sound: Option<(u32, SoundDecoder)>,
    picture: Option<(u32, PictureDecoder)>,
    resampler: Option<media::Resampler>,
    /// The sound's frames before `from` are dropped: a seek's.
    from: f64,
}

impl Decoders {
    fn reset(&mut self, from: f64) {
        if let Some((_, decoder)) = &mut self.sound {
            decoder.reset();
        }
        if let Some((_, decoder)) = &mut self.picture {
            let _ = decoder.reset();
        }
        self.resampler = None;
        self.from = from;
    }
}

/// The sound's samples as the output's: stereo (the first two
/// channels, or one on both), at its rate scaled by `rate`.
fn to_output(
    sound: &media::Sound,
    from: f64,
    rate: f64,
    resampler: &mut Option<media::Resampler>,
) -> Vec<f32> {
    let channels = usize::from(sound.channels.max(1));
    let frames = sound.samples.len() / channels;
    let skip = (((from - sound.time) * f64::from(sound.rate))
        .ceil()
        .max(0.0) as usize)
        .min(frames);
    let mut stereo = Vec::with_capacity((frames - skip) * OUTPUT_CHANNELS);
    for frame in sound.samples.chunks_exact(channels).skip(skip) {
        let left = frame[0];
        let right = if channels > 1 { frame[1] } else { frame[0] };
        stereo.push(left);
        stereo.push(right);
    }
    // At the output's rate already: as it is.
    if f64::from(sound.rate) * rate == f64::from(OUTPUT_RATE) {
        return stereo;
    }
    let resampler = resampler.get_or_insert_with(|| {
        media::Resampler::new(
            f64::from(sound.rate) * rate,
            f64::from(OUTPUT_RATE),
            OUTPUT_CHANNELS,
        )
    });
    resampler.process(&stereo)
}

fn run(shared: &Shared, resource: &MediaResource, hint: &Hint, tell: &dyn Fn(Report)) {
    // The head first: the fetch at rest, or enough of it.
    loop {
        if lock(shared).stopped {
            return;
        }
        if let Some(MediaError::Network(_)) = resource.failed() {
            tell(Report::Failed(format_error()));
            return;
        }
        if resource.idle() || resource.bytes_held() as u64 >= HEAD {
            break;
        }
        std::thread::sleep(LOOK);
    }
    // Another origin's, without CORS: a graph hears silence of it.
    let tainted = resource
        .head()
        .is_some_and(|head| head.response_type == net::ResponseType::Opaque);
    shared.sound.tainted.store(tainted, Ordering::Relaxed);
    tell(Report::Fetched);
    let mut demuxer = match Demuxer::open(Box::new(resource.reader()), hint) {
        Ok(demuxer) => demuxer,
        Err(_) => {
            tell(Report::Failed(format_error()));
            return;
        }
    };
    let sound = demuxer
        .tracks()
        .iter()
        .find(|t| t.is_sound() && t.decodable())
        .and_then(|t| SoundDecoder::new(t).ok().map(|d| (t.id, d)));
    let picture = demuxer
        .tracks()
        .iter()
        .find(|t| t.is_picture() && t.decodable())
        .and_then(|t| PictureDecoder::new(t).ok().map(|d| (t.id, d)));
    if sound.is_none() && picture.is_none() {
        tell(Report::Failed(format_error()));
        return;
    }
    let size = demuxer.tracks().iter().find_map(|t| match t.kind {
        media::TrackKind::Picture { width, height, .. }
            if picture.as_ref().is_some_and(|(id, _)| *id == t.id) =>
        {
            Some((width, height))
        }
        _ => None,
    });
    let duration = demuxer.duration().unwrap_or(f64::INFINITY);
    let mut decoders = Decoders {
        sound,
        picture,
        resampler: None,
        from: 0.0,
    };
    let meta = Meta {
        duration,
        size,
        sound: decoders.sound.is_some(),
    };
    lock(shared).meta = Some(meta.clone());
    // A resource of sound alone tells its head before its first sound.
    if decoders.picture.is_none() {
        tell(Report::Meta(meta.clone()));
    }
    // The first picture, or sound, before the element hears of it.
    let mut told_ready = false;
    let mut last_time = Instant::now();
    loop {
        let mut state = lock(shared);
        if state.stopped {
            return;
        }
        // A seek: the demuxer to its packet, the queues emptied, the
        // decoders made again, and decoding forward to its time.
        if let Some((id, time)) = state.seek.take() {
            state.pictures.clear();
            state.drained = false;
            state.ended = false;
            state.dry = false;
            shared
                .sound
                .queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear();
            shared.set_clock(&mut state, time);
            drop(state);
            let track = decoders
                .picture
                .as_ref()
                .map(|(id, _)| *id)
                .or(decoders.sound.as_ref().map(|(id, _)| *id))
                .unwrap_or(0);
            let _ = demuxer.seek(time, track);
            decoders.reset(time);
            fill(shared, &mut demuxer, &mut decoders, Some(time));
            if id != 0 {
                tell(Report::Seeked(id, time));
            }
            continue;
        }
        let playing = state.playing;
        // The pictures the clock passed go, the one shown kept.
        let now = shared.position(&state);
        while state.pictures.len() > 1 && state.pictures[1].time <= now + 1e-6 {
            state.pictures.pop_front();
        }
        let full_pictures =
            decoders.picture.is_none() || state.pictures.len() >= MAX_DECODED_PICTURES;
        let full_sound = decoders.sound.is_none()
            || shared
                .sound
                .queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .len()
                >= MAX_DECODED_SOUND * OUTPUT_CHANNELS;
        let drained = state.drained;
        // The end: decoded all, and the clock past the last of it.
        if playing && drained && !state.ended {
            let sound_left = !shared
                .sound
                .queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_empty();
            // The sound all played before the end: the system's clock on.
            if meta.sound && !sound_left && !state.dry {
                let now = shared.position(&state);
                state.dry = true;
                shared.set_clock(&mut state, now);
            }
            let now = shared.position(&state);
            let at_end = if meta.duration.is_finite() {
                now >= meta.duration - 1e-3
                    || (!sound_left && meta.sound && decoders.picture.is_none())
            } else {
                !sound_left
            };
            if at_end {
                let end = if meta.duration.is_finite() {
                    meta.duration
                } else {
                    now
                };
                state.ended = true;
                state.playing = false;
                shared.sound.playing.store(false, Ordering::Relaxed);
                shared.set_clock(&mut state, end);
                drop(state);
                tell(Report::Ended(end));
                continue;
            }
        }
        if playing && last_time.elapsed() >= TIME_UPDATE {
            last_time = Instant::now();
            let now = shared.position(&state);
            drop(state);
            tell(Report::Time(now));
            continue;
        }
        if !playing {
            last_time = Instant::now();
        }
        if drained || (full_pictures && full_sound) {
            let _ = shared
                .changed
                .wait_timeout(state, LOOK)
                .unwrap_or_else(PoisonError::into_inner);
            continue;
        }
        drop(state);
        fill(shared, &mut demuxer, &mut decoders, None);
        if !told_ready {
            let state = lock(shared);
            // Ready: the first picture, and enough sound to play on with.
            let pictured = decoders.picture.is_none() || !state.pictures.is_empty();
            let sounded = decoders.sound.is_none()
                || shared
                    .sound
                    .queue
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .len()
                    >= READY_SOUND * OUTPUT_CHANNELS;
            let first = pictured && sounded;
            if first || state.drained {
                let mut state = state;
                // Playback, if asked for, starts now, its clock with it.
                state.ready = true;
                let now = shared.position(&state);
                shared.set_clock(&mut state, now);
                shared.sound.playing.store(state.playing, Ordering::Relaxed);
                drop(state);
                told_ready = true;
                tell(Report::Ready(meta.clone()));
            }
        }
    }
}

/// Decodes a packet ahead, or, after a seek to `to`, until the picture
/// and the sound at `to` are there.
fn fill(shared: &Shared, demuxer: &mut Demuxer, decoders: &mut Decoders, to: Option<f64>) {
    let mut have_picture = decoders.picture.is_none();
    let mut have_sound = decoders.sound.is_none();
    loop {
        let packet = match demuxer.next_packet() {
            Ok(Some(packet)) => packet,
            _ => {
                if let Some((_, decoder)) = &mut decoders.picture
                    && let Ok(pictures) = decoder.drain()
                {
                    let mut state = lock(shared);
                    state.pictures.extend(pictures);
                }
                lock(shared).drained = true;
                return;
            }
        };
        if let Some((id, decoder)) = &mut decoders.picture
            && *id == packet.track
            && let Ok(pictures) = decoder.decode(&packet)
        {
            let mut state = lock(shared);
            for picture in pictures {
                // After a seek, the pictures before its time but the
                // last are passed.
                if let Some(to) = to
                    && picture.time < to - 1e-6
                {
                    state.pictures.clear();
                }
                if to.is_none_or(|to| picture.time + 1e-6 >= to) {
                    have_picture = true;
                }
                state.pictures.push_back(picture);
            }
        }
        if let Some((id, decoder)) = &mut decoders.sound
            && *id == packet.track
            && let Ok(sound) = decoder.decode(&packet)
        {
            let rate = lock(shared).rate;
            let samples = to_output(&sound, decoders.from, rate, &mut decoders.resampler);
            if !samples.is_empty() {
                have_sound = true;
            }
            shared
                .sound
                .queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(samples);
        }
        if to.is_none() || (have_picture && have_sound) {
            return;
        }
    }
}

impl Report {
    /// The report as it crosses to the document's task.
    pub(crate) fn to_json(&self) -> devtools::Json {
        use devtools::Json;
        let meta = |meta: &Meta| {
            Json::object([
                (
                    "duration",
                    Json::from(if meta.duration.is_finite() {
                        meta.duration
                    } else {
                        -1.0
                    }),
                ),
                ("width", Json::from(u64::from(meta.size.map_or(0, |s| s.0)))),
                (
                    "height",
                    Json::from(u64::from(meta.size.map_or(0, |s| s.1))),
                ),
                ("video", Json::Bool(meta.size.is_some())),
                ("sound", Json::Bool(meta.sound)),
            ])
        };
        let kind = |kind: &str| ("kind", Json::from(kind));
        match self {
            Report::Fetched => Json::object([kind("fetched")]),
            Report::Meta(m) => Json::object([kind("meta"), ("meta", meta(m))]),
            Report::Ready(m) => Json::object([kind("ready"), ("meta", meta(m))]),
            Report::Seeked(id, time) => Json::object([
                kind("seeked"),
                ("id", Json::from(*id)),
                ("time", Json::from(*time)),
            ]),
            Report::Time(time) => Json::object([kind("time"), ("time", Json::from(*time))]),
            Report::Ended(time) => Json::object([kind("ended"), ("time", Json::from(*time))]),
            Report::Failed(failure) => Json::object([
                kind("failed"),
                ("code", Json::from(u64::from(failure.code))),
                ("message", Json::from(failure.message.as_str())),
            ]),
        }
    }

    pub(crate) fn from_json(json: &devtools::Json) -> Option<Report> {
        use devtools::Json;
        let number = |name: &str| json.get(name).and_then(Json::as_f64);
        let meta = || {
            let meta = json.get("meta")?;
            let get = |name: &str| meta.get(name).and_then(Json::as_f64);
            let duration = get("duration")?;
            let video = meta.get("video").and_then(Json::as_bool)?;
            Some(Meta {
                duration: if duration < 0.0 {
                    f64::INFINITY
                } else {
                    duration
                },
                size: video.then(|| {
                    (
                        get("width").unwrap_or(0.0) as u32,
                        get("height").unwrap_or(0.0) as u32,
                    )
                }),
                sound: meta.get("sound").and_then(Json::as_bool)?,
            })
        };
        Some(match json.get("kind").and_then(Json::as_str)? {
            "fetched" => Report::Fetched,
            "meta" => Report::Meta(meta()?),
            "ready" => Report::Ready(meta()?),
            "seeked" => Report::Seeked(number("id")? as u64, number("time")?),
            "time" => Report::Time(number("time")?),
            "ended" => Report::Ended(number("time")?),
            "failed" => Report::Failed(Failure {
                code: number("code")? as u16,
                message: json.get("message").and_then(Json::as_str)?.to_owned(),
            }),
            _ => return None,
        })
    }
}
