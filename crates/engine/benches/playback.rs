//! M40's benchmark: a video's frame and a Web Audio graph's quantum.
//!
//! - **A picture decoded and converted.** The AV1 clip of the media
//!   crate's fixtures (160 by 120, 48 pictures), each picture decoded by
//!   `rav1d` and converted to RGBA (`PictureDecoder::decode`): the time a
//!   picture.
//! - **A quantum rendered.** A graph of eight oscillators through a
//!   biquad, a gain and a stereo panner to the destination, at 48 kHz:
//!   the time a quantum of 128 frames, which real time gives 2.67 ms.
//! - **A frame of a page playing a video.** The clip, muted and playing,
//!   at 320 by 240 on a page of 800 by 600: each frame drawn, the picture
//!   at the media clock given to paint, for a second of playback.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench playback
//! ```

#![forbid(unsafe_code)]

use std::hint::black_box;
use std::time::{Duration, Instant};

use engine::{Engine, LoadState};
use net::test_server::{Reply, ServerRequest, TestServer};

const CLIP: &[u8] = include_bytes!("../../media/fixtures/clip-video.webm");
const RUNS: usize = 20;

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// The median and range of `times`, in milliseconds.
fn spread(times: &mut [f64]) -> String {
    times.sort_by(f64::total_cmp);
    format!(
        "median {:.3} ms, {:.3}-{:.3} ms",
        times[times.len() / 2],
        times[0],
        times[times.len() - 1]
    )
}

/// Each picture of the clip decoded and converted, `RUNS` times: the
/// times a picture.
fn pictures() -> Vec<f64> {
    let mut times = Vec::new();
    for _ in 0..RUNS {
        let source = Box::new(media::Bytes::new(CLIP.to_vec()));
        let mut demuxer = media::Demuxer::open(source, &media::Hint::default()).expect("the clip");
        let track = demuxer.first_picture().expect("a picture track").clone();
        let mut decoder = media::PictureDecoder::new(&track).expect("a decoder");
        let mut count = 0;
        let started = Instant::now();
        while let Ok(Some(packet)) = demuxer.next_packet() {
            if packet.track != track.id {
                continue;
            }
            count += black_box(decoder.decode(&packet).expect("decoded")).len();
        }
        count += decoder.drain().expect("drained").len();
        times.push(ms(started.elapsed()) / count.max(1) as f64);
    }
    times
}

/// A quantum of the graph, `RUNS` times a thousand quanta: the times a
/// quantum.
fn quanta() -> Vec<f64> {
    use audio::nodes;
    let mut times = Vec::new();
    for _ in 0..RUNS {
        let mut graph = audio::Graph::new(48_000.0, 2);
        let filter = graph.add(nodes::biquad(48_000.0)).expect("a node");
        let gain = graph.add(nodes::gain()).expect("a node");
        let pan = graph.add(nodes::stereo_panner()).expect("a node");
        for i in 0..8 {
            let mut oscillator = nodes::oscillator(48_000.0);
            oscillator.params[0].value = 110.0 * (i + 1) as f32;
            if let nodes::Kind::Oscillator(o) = &mut oscillator.kind {
                o.schedule.start = Some(0.0);
            }
            let id = graph.add(oscillator).expect("a node");
            graph.connect(id, 0, filter, 0);
        }
        graph.connect(filter, 0, gain, 0);
        graph.connect(gain, 0, pan, 0);
        graph.connect(pan, 0, 0, 0);
        let started = Instant::now();
        for _ in 0..1000 {
            black_box(graph.render());
        }
        times.push(ms(started.elapsed()) / 1000.0);
    }
    times
}

/// The clip as a server answering ranges answers `request`.
fn clip(request: &ServerRequest) -> Reply {
    let first = request
        .header("Range")
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.split('-').next()?.parse::<usize>().ok())
        .unwrap_or(0)
        .min(CLIP.len() - 1);
    Reply::with_fields(
        206,
        &[
            ("Content-Type", "video/webm"),
            (
                "Content-Range",
                &format!("bytes {first}-{}/{}", CLIP.len() - 1, CLIP.len()),
            ),
        ],
        &CLIP[first..],
    )
}

/// The frames of a second of the page playing the clip: the times a
/// frame, and how many showed the video changed from the frame before
/// (the clip is red for a second and green for the next: one or two).
fn frames() -> (Vec<f64>, usize) {
    let server = TestServer::plain(|request| {
        match request.target.as_str() {
        "/page.html" => Reply::ok(
            "text/html",
            b"<body style=margin:0><video src=/clip.webm muted autoplay loop style=width:320px></video>",
        ),
        "/clip.webm" => clip(request),
        _ => Reply::with_fields(404, &[], b""),
    }
    });
    let mut engine = Engine::new().expect("an engine");
    engine.navigate(server.url("/page.html"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while *engine.load_state() != LoadState::Loaded && Instant::now() < deadline {
        engine.poll_network();
        engine.run_tasks();
        std::thread::sleep(Duration::from_millis(5));
    }
    // Its first picture, before the second is timed.
    let ready = Instant::now() + Duration::from_millis(500);
    while Instant::now() < ready {
        engine.poll_network();
        engine.run_tasks();
        engine.frame(800, 600);
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut times = Vec::new();
    let mut changed = 0;
    let mut last: Option<Vec<u8>> = None;
    let end = Instant::now() + Duration::from_secs(1);
    while Instant::now() < end {
        engine.poll_network();
        engine.run_tasks();
        let started = Instant::now();
        let pixels = engine.frame(800, 600);
        times.push(ms(started.elapsed()));
        // The video's rectangle, 320 by 240 at the page's corner.
        let now: Vec<u8> = (0..240)
            .flat_map(|y| pixels[y * 800 * 4..(y * 800 + 320) * 4].iter().copied())
            .collect();
        if last.as_ref() != Some(&now) {
            changed += 1;
        }
        last = Some(now);
        std::thread::sleep(Duration::from_millis(16));
    }
    (times, changed)
}

fn main() {
    let mut times = pictures();
    println!(
        "an AV1 picture decoded and converted (160 by 120): {}",
        spread(&mut times)
    );
    let mut times = quanta();
    println!(
        "a quantum of eight oscillators, a biquad, a gain and a panner: {}",
        spread(&mut times)
    );
    let (mut times, changed) = frames();
    let count = times.len();
    println!(
        "a frame of a page playing the clip at 320 by 240: {} ({count} frames, the video changed in {changed})",
        spread(&mut times)
    );
}
