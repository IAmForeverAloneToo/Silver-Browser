//! Media elements (M40 step 6): their events and states as the reference
//! browser showed them for the same clips (scratchpad `m40/pages/p2`,
//! `p4`, Chrome 155): loading, seeking, playing to the end; the autoplay
//! policy; errors, `<source>` selection, `canPlayType()`; and their
//! sound reaching the output.

use net::test_server::{Reply, ServerRequest, TestServer};

use crate::testing::*;

const CLIP: &[u8] = include_bytes!("../../../media/fixtures/clip.webm");
const CLIP_MP4: &[u8] = include_bytes!("../../../media/fixtures/clip.mp4");
const CLIP_VIDEO: &[u8] = include_bytes!("../../../media/fixtures/clip-video.webm");
const CLIP_AUDIO: &[u8] = include_bytes!("../../../media/fixtures/clip-audio.webm");
const CLIP_OGG: &[u8] = include_bytes!("../../../media/fixtures/clip.opus.ogg");
const TONE: &[u8] = include_bytes!("../../../media/fixtures/tone.wav");

/// `data` as a server answering ranges answers `request`.
fn ranged(request: &ServerRequest, kind: &str, data: &[u8]) -> Reply {
    let range = request
        .header("Range")
        .and_then(|range| range.strip_prefix("bytes="))
        .and_then(|range| range.split_once('-'));
    let Some((first, last)) = range else {
        return Reply::ok(kind, data);
    };
    let first: usize = first.parse().unwrap_or(0);
    let last = last
        .parse::<usize>()
        .unwrap_or(data.len() - 1)
        .min(data.len() - 1);
    if first > last {
        return Reply::with_fields(416, &[], b"");
    }
    Reply::with_fields(
        206,
        &[
            ("Content-Type", kind),
            (
                "Content-Range",
                &format!("bytes {first}-{last}/{}", data.len()),
            ),
        ],
        &data[first..=last],
    )
}

fn server(script: &str) -> TestServer {
    let body = format!(
        "<style>body {{ margin: 0 }} #hit {{ width: 100px; height: 100px }}</style><div id=hit></div>\
         <script>const log = (s) => console.log(s);\n{script}</script>"
    );
    TestServer::plain(move |request| match request.target.as_str() {
        "/page.html" => html(&body),
        "/media/clip.webm" => ranged(request, "video/webm", CLIP),
        "/media/clip.mp4" => ranged(request, "video/mp4", CLIP_MP4),
        "/media/clip-video.webm" => ranged(request, "video/webm", CLIP_VIDEO),
        "/media/clip-audio.webm" => ranged(request, "audio/webm", CLIP_AUDIO),
        "/media/clip.opus.ogg" => ranged(request, "audio/ogg", CLIP_OGG),
        "/media/tone.wav" => ranged(request, "audio/wav", TONE),
        "/media/page.txt" => Reply::ok("text/plain", b"not media at all"),
        _ => Reply::with_fields(404, &[], b""),
    })
}

/// Opens the page, presses the pointer on it (the user's activation) when
/// `activate`, and runs it until it logs "done".
fn run(page: &TestServer, activate: bool) -> Vec<String> {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(page.url("/page.html"));
    let mut lines = tab.run_until(|lines| lines.iter().any(|l| l == "ready"));
    if activate {
        tab.engine
            .key_down(&crate::Key::new("a", "KeyA").with_text("a"));
    }
    if !lines.iter().any(|l| l == "done") {
        lines.extend(
            tab.run_until(|lines| lines.iter().any(|l| l == "done" || l.starts_with("failed"))),
        );
    }
    lines
        .into_iter()
        .filter(|l| l != "done" && l != "ready")
        .collect()
}

/// Probe p2's watcher: each event with the states it saw.
const WATCH: &str = r#"
const EVENTS = ['loadstart','progress','suspend','abort','error','emptied','stalled','loadedmetadata','loadeddata','canplay','canplaythrough','playing','waiting','seeking','seeked','ended','durationchange','timeupdate','play','pause','ratechange','resize','volumechange'];
function watch(m) {
  const seen = [];
  for (const e of EVENTS) m.addEventListener(e, () => seen.push(e + '(' + m.readyState + ',' + m.networkState + ')'));
  return seen;
}
const wait = (m, e) => new Promise(r => m.addEventListener(e, r, {once: true}));
"#;

#[test]
fn a_video_loads_seeks_and_plays_to_its_end_as_the_reference_shows() {
    // It plays: the process's mixer is the test's while it does.
    let _turn = audio_turn();
    let page = server(&format!(
        "{WATCH}
log('ready');
addEventListener('keydown', async () => {{
  try {{
  for (const f of ['clip.webm', 'clip.mp4', 'clip-video.webm']) {{
    const v = document.createElement('video'); document.body.appendChild(v);
    const seen = watch(v);
    v.src = '/media/' + f;
    await wait(v, 'canplaythrough');
    await new Promise(r => setTimeout(r, 200));
    log(f + ' load: ' + seen.join(' '));
    log(f + ' duration ' + v.duration + ' size ' + v.videoWidth + 'x' + v.videoHeight + ' buffered ' + (v.buffered.length ? v.buffered.start(0) + '-' + v.buffered.end(v.buffered.length - 1) : '-') + ' seekable ' + (v.seekable.length ? v.seekable.end(0) : '-') + ' currentSrc ' + v.currentSrc.replace(location.origin, ''));
    seen.length = 0;
    v.currentTime = 1.5; await wait(v, 'seeked');
    log(f + ' seek: ' + seen.join(' ') + ' t=' + v.currentTime);
    seen.length = 0;
    v.currentTime = 0.5; await wait(v, 'seeked');
    seen.length = 0;
    const p = v.play(); await p.then(() => log(f + ' play resolved'), e => log(f + ' play rejected ' + e.name + ': ' + e.message));
    await wait(v, 'ended');
    log(f + ' play: ' + seen.filter(e => !e.startsWith('timeupdate')).join(' ') + ' t=' + v.currentTime + ' paused ' + v.paused + ' ended ' + v.ended + ' played ' + v.played.length + ':' + v.played.start(0).toFixed(2) + '-' + v.played.end(0).toFixed(3));
    v.remove();
  }}
  }} catch (e) {{ log('failed ' + e + ' ' + e.stack); }}
  log('done');
}}, {{ once: true }});
"
    ));
    let lines = run(&page, true);
    let mut wanted = Vec::new();
    for (f, duration) in [
        ("clip.webm", "2.1"),
        ("clip.mp4", "2.02"),
        ("clip-video.webm", "2"),
    ] {
        let end = format!("{:.3}", duration.parse::<f64>().unwrap());
        wanted.extend([
            format!("{f} load: loadstart(0,2) progress(0,1) suspend(0,1) durationchange(4,1) resize(4,1) loadedmetadata(4,1) loadeddata(4,1) canplay(4,1) canplaythrough(4,1)"),
            format!("{f} duration {duration} size 160x120 buffered 0-{duration} seekable {duration} currentSrc /media/{f}"),
            format!("{f} seek: seeking(1,1) timeupdate(4,1) seeked(4,1) t=1.5"),
            format!("{f} play resolved"),
            format!("{f} play: canplay(4,1) canplaythrough(4,1) play(4,1) playing(4,1) pause(4,1) ended(4,1) t={duration} paused true ended true played 1:0.50-{end}"),
        ]);
    }
    assert_eq!(lines, wanted);
}

#[test]
fn sound_alone_loads_as_the_reference_shows() {
    let page = server(&format!(
        "{WATCH}
log('ready');
(async () => {{
  for (const f of ['clip-audio.webm', 'clip.opus.ogg', 'tone.wav']) {{
    const a = new Audio('/media/' + f); const seen = watch(a);
    await wait(a, 'canplaythrough');
    log(f + ' audio load: ' + seen.join(' ') + ' duration ' + a.duration + ' size ' + a.videoWidth);
  }}
  log('done');
}})().catch(e => log('failed ' + e));
"
    ));
    let lines = run(&page, false);
    let load = "loadstart(0,2) progress(0,1) suspend(0,1) durationchange(1,1) loadedmetadata(1,1) loadeddata(4,1) canplay(4,1) canplaythrough(4,1)";
    assert_eq!(
        lines,
        [
            format!("clip-audio.webm audio load: {load} duration 2.02 size undefined"),
            format!("clip.opus.ogg audio load: {load} duration 2.0065 size undefined"),
            format!("tone.wav audio load: {load} duration 1 size undefined"),
        ]
    );
}

/// Probe p4's calls, but the boxes (step 7): the autoplay policy, the
/// errors, `<source>` selection, and the setters' checks.
const P4: &str = r#"
const wait = (m, e) => new Promise(r => m.addEventListener(e, r, {once: true}));
const run = async () => {
  const a = document.createElement('video'); a.src = '/media/clip.webm';
  await a.play().then(() => log('unmuted play resolved'), e => log('unmuted play: ' + e.name + ': ' + e.message));
  const m = document.createElement('video'); m.src = '/media/clip.webm'; m.muted = true;
  await m.play().then(() => log('muted play resolved'), e => log('muted play: ' + e.name + ': ' + e.message));
  const ap = document.createElement('video'); ap.autoplay = true; ap.src = '/media/clip.webm';
  await new Promise(r => setTimeout(r, 1500)); log('autoplay unmuted paused=' + ap.paused + ' t>0=' + (ap.currentTime > 0));
  const n = document.createElement('video'); n.src = '/media/missing.webm';
  await wait(n, 'error'); log('404: code ' + n.error.code + ' ' + JSON.stringify(n.error.message) + ' networkState ' + n.networkState + ' readyState ' + n.readyState);
  await n.play().then(() => log('?'), e => log('play after 404: ' + e.name + ': ' + e.message));
  const t = document.createElement('video'); t.src = '/media/page.txt';
  await wait(t, 'error'); log('text: code ' + t.error.code + ' ' + JSON.stringify(t.error.message));
  const e2 = document.createElement('video'); e2.src = '';
  await new Promise(r => setTimeout(r, 300)); log('empty src: ' + (e2.error && e2.error.code) + ' ' + (e2.error && JSON.stringify(e2.error.message)) + ' ns ' + e2.networkState);
  const s = document.createElement('video');
  s.innerHTML = '<source src="/media/missing.webm" type="video/webm"><source src="/media/x.mp4" type="video/quicktime"><source src="/media/clip-video.webm">';
  document.body.appendChild(s); await wait(s, 'loadedmetadata'); log('source pick: ' + s.currentSrc.replace(location.origin, ''));
  const s2 = document.createElement('video'); s2.innerHTML = '<source src="/media/missing.webm">';
  document.body.appendChild(s2);
  s2.querySelector('source').addEventListener('error', () => log('source error event at <source>, media error ' + s2.error + ' ns ' + s2.networkState));
  await new Promise(r => setTimeout(r, 800));
  let s2p = 'pending'; s2.play().then(() => s2p = 'resolved', e => s2p = e.name + ': ' + e.message); await new Promise(r => setTimeout(r, 500)); log('play no source: ' + s2p);
  const p = document.createElement('video'); p.src = '/media/clip.webm'; const pp = p.play(); p.pause();
  await pp.then(() => log('?'), e => log('play then pause: ' + e.name + ': ' + e.message));
  const l = document.createElement('video'); l.src = '/media/clip.webm'; const lp = l.play(); l.src = '/media/clip.mp4';
  await lp.then(() => log('?'), e => log('play then load: ' + e.name + ': ' + e.message));
  try { new Audio().currentTime = -1; log('neg ok'); } catch (e) { log('neg: ' + e.name); }
  try { const q = new Audio(); q.volume = 2; } catch (e) { log('volume 2: ' + e.name + ': ' + e.message); }
  try { const q = new Audio(); q.playbackRate = 20; log('rate 20 ok'); } catch (e) { log('rate 20: ' + e.name + ': ' + e.message); }
  log('done');
};
"#;

const NOT_ALLOWED: &str = "NotAllowedError: play() failed because the user didn't interact with the document first. https://goo.gl/xX8pDD";
const VOLUME_2: &str = "volume 2: IndexSizeError: Failed to set the 'volume' property on 'HTMLMediaElement': The volume provided (2) is outside the range [0, 1].";
const RATE_20: &str = "rate 20: NotSupportedError: Failed to set the 'playbackRate' property on 'HTMLMediaElement': The provided playback rate (20) is not in the supported playback range.";

#[test]
fn without_the_users_activation_sound_does_not_start_as_the_reference_refuses_it() {
    // It plays: the process's mixer is the test's while it does.
    let _turn = audio_turn();
    let page = server(&format!(
        "{P4}\nlog('ready'); run().catch(e => log('failed ' + e));"
    ));
    let lines = run(&page, false);
    assert_eq!(
        lines,
        [
            format!("unmuted play: {NOT_ALLOWED}"),
            "muted play resolved".to_owned(),
            "autoplay unmuted paused=true t>0=false".to_owned(),
            "404: code 4 \"MEDIA_ELEMENT_ERROR: Format error\" networkState 3 readyState 0"
                .to_owned(),
            format!("play after 404: {NOT_ALLOWED}"),
            "text: code 4 \"MEDIA_ELEMENT_ERROR: Format error\"".to_owned(),
            "empty src: 4 \"MEDIA_ELEMENT_ERROR: Empty src attribute\" ns 3".to_owned(),
            "source pick: /media/clip-video.webm".to_owned(),
            "source error event at <source>, media error null ns 3".to_owned(),
            format!("play no source: {NOT_ALLOWED}"),
            format!("play then pause: {NOT_ALLOWED}"),
            format!("play then load: {NOT_ALLOWED}"),
            "neg ok".to_owned(),
            VOLUME_2.to_owned(),
            RATE_20.to_owned(),
        ]
    );
}

#[test]
fn with_the_users_activation_sound_starts_as_the_reference_lets_it() {
    // It plays: the process's mixer is the test's while it does.
    let _turn = audio_turn();
    let page = server(&format!(
        "{P4}\nlog('ready'); addEventListener('keydown', () => run().catch(e => log('failed ' + e)), {{ once: true }});"
    ));
    let lines = run(&page, true);
    assert_eq!(
        lines,
        [
            "unmuted play resolved",
            "muted play resolved",
            "autoplay unmuted paused=false t>0=true",
            "404: code 4 \"MEDIA_ELEMENT_ERROR: Format error\" networkState 3 readyState 0",
            "play after 404: NotSupportedError: The element has no supported sources.",
            "text: code 4 \"MEDIA_ELEMENT_ERROR: Format error\"",
            "empty src: 4 \"MEDIA_ELEMENT_ERROR: Empty src attribute\" ns 3",
            "source pick: /media/clip-video.webm",
            "source error event at <source>, media error null ns 3",
            "play no source: pending",
            "play then pause: AbortError: The play() request was interrupted by a call to pause(). https://goo.gl/LdLk22",
            "play then load: AbortError: The play() request was interrupted by a new load request. https://goo.gl/LdLk22",
            "neg ok",
            VOLUME_2,
            RATE_20,
        ]
    );
}

/// The mixer writes to `sink` while this lives; then to a sink that
/// plays at the clock again.
struct Sink;

impl Sink {
    fn set(sink: std::sync::Arc<dyn crate::AudioSink>) -> Sink {
        crate::audio_output::set_audio_sink(sink);
        Sink
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        crate::audio_output::set_audio_sink(std::sync::Arc::new(
            crate::audio_output::ClockSink::new(),
        ));
    }
}

/// The left channel's zero crossings and peak, and the right's peak, of
/// interleaved stereo `samples`.
fn heard(samples: &[f32]) -> (usize, f32, f32) {
    let left: Vec<f32> = samples.iter().step_by(2).copied().collect();
    let right = samples
        .iter()
        .skip(1)
        .step_by(2)
        .fold(0.0f32, |m, s| m.max(s.abs()));
    let crossings = left
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    (
        crossings,
        left.iter().fold(0.0f32, |m, s| m.max(s.abs())),
        right,
    )
}

#[test]
fn an_elements_sound_reaches_the_output_or_a_graph_that_takes_it() {
    let _turn = audio_turn();
    let capture = std::sync::Arc::new(Capture::paced());
    let _sink = Sink::set(capture.clone());
    let page = server(
        r#"
const wait = (m, e) => new Promise(r => m.addEventListener(e, r, {once: true}));
log('ready');
addEventListener('keydown', async () => {
  const a = new Audio('/media/clip-audio.webm');
  await a.play();
  await wait(a, 'ended');
  log('played');
  await new Promise(r => addEventListener('keydown', r, { once: true }));
  const b = new Audio('/media/clip-audio.webm');
  const ctx = new AudioContext();
  const source = ctx.createMediaElementSource(b);
  log(source.mediaElement === b && source.numberOfInputs + ',' + source.numberOfOutputs);
  try { ctx.createMediaElementSource(b); } catch (e) { log(e.name + ': ' + e.message); }
  log(typeof OfflineAudioContext.prototype.createMediaElementSource);
  const g = ctx.createGain(); g.gain.value = 0.5;
  source.connect(g).connect(ctx.destination);
  await b.play();
  await wait(b, 'ended');
  log('done');
}, { once: true });
"#,
    );
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(page.url("/page.html"));
    tab.run_until(|lines| lines.iter().any(|l| l == "ready"));
    tab.engine
        .key_down(&crate::Key::new("a", "KeyA").with_text("a"));
    let lines = tab.run_until(|lines| lines.iter().any(|l| l == "played"));
    assert_eq!(lines, ["played"]);
    // The clip: 440 Hz on the left at about half scale, the right silent.
    let first = capture.samples.lock().unwrap().clone();
    let start = first.iter().position(|s| s.abs() > 0.1).expect("sound") & !1;
    let (crossings, left, right) = heard(&first[start..start + 9600 * 2]);
    assert!((173..=179).contains(&crossings), "{crossings}");
    assert!((0.50..=0.53).contains(&left), "{left}");
    assert!(right < 1e-3, "{right}");
    capture.samples.lock().unwrap().clear();
    tab.engine
        .key_down(&crate::Key::new("a", "KeyA").with_text("a"));
    let lines = tab.run_until(|lines| lines.iter().any(|l| l == "done" || l.starts_with("failed")));
    assert_eq!(
        lines,
        [
            "0,1",
            "InvalidStateError: Failed to execute 'createMediaElementSource' on 'AudioContext': HTMLMediaElement already connected previously to a different MediaElementSourceNode.",
            "undefined",
            "done",
        ]
    );
    // Through the graph, at half the gain.
    let second = capture.samples.lock().unwrap().clone();
    let start = second.iter().position(|s| s.abs() > 0.05).expect("sound") & !1;
    let (crossings, left, _) = heard(&second[start..start + 9600 * 2]);
    assert!((173..=179).contains(&crossings), "{crossings}");
    assert!((0.25..=0.27).contains(&left), "{left}");
}

#[test]
fn can_play_type_answers_as_the_reference_does() {
    let page = server(
        r#"
log('ready');
const v = document.createElement('video');
for (const t of ['video/webm', 'video/webm; codecs="av01.0.04M.08, opus"', 'video/webm; codecs="vp9, opus"', 'video/mp4', 'video/mp4; codecs="flac"', 'audio/ogg; codecs="opus"', 'audio/mpeg', 'audio/wav', 'audio/wav; codecs="1"', 'audio/wave', 'audio/x-flac', 'audio/flac', 'video/ogg; codecs="theora"', 'video/quicktime', 'text/plain', '', 'video/webm; codecs="bogus"', 'audio/opus'])
  log(JSON.stringify(t) + ' = ' + JSON.stringify(v.canPlayType(t)));
log([HTMLMediaElement.HAVE_ENOUGH_DATA, v.NETWORK_NO_SOURCE, MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED].join());
const a = new Audio();
log(a.tagName + ' preload=' + a.preload + ' ' + Object.prototype.toString.call(a) + ' ' + (a instanceof HTMLAudioElement));
log(JSON.stringify({readyState: v.readyState, networkState: v.networkState, paused: v.paused, volume: v.volume, muted: v.muted, preload: v.preload, playbackRate: v.playbackRate, currentTime: v.currentTime, duration: v.duration, seeking: v.seeking, ended: v.ended, error: v.error, buffered: v.buffered.length, preservesPitch: v.preservesPitch, crossOrigin: v.crossOrigin}));
try { v.buffered.start(0); } catch (e) { log(e.name + ': ' + e.message); }
log('done');
"#,
    );
    let lines = run(&page, false);
    assert_eq!(
        lines,
        [
            "\"video/webm\" = \"maybe\"",
            "\"video/webm; codecs=\\\"av01.0.04M.08, opus\\\"\" = \"probably\"",
            "\"video/webm; codecs=\\\"vp9, opus\\\"\" = \"\"",
            "\"video/mp4\" = \"maybe\"",
            "\"video/mp4; codecs=\\\"flac\\\"\" = \"probably\"",
            "\"audio/ogg; codecs=\\\"opus\\\"\" = \"probably\"",
            "\"audio/mpeg\" = \"probably\"",
            "\"audio/wav\" = \"maybe\"",
            "\"audio/wav; codecs=\\\"1\\\"\" = \"probably\"",
            "\"audio/wave\" = \"\"",
            "\"audio/x-flac\" = \"\"",
            "\"audio/flac\" = \"probably\"",
            "\"video/ogg; codecs=\\\"theora\\\"\" = \"\"",
            "\"video/quicktime\" = \"\"",
            "\"text/plain\" = \"\"",
            "\"\" = \"\"",
            "\"video/webm; codecs=\\\"bogus\\\"\" = \"\"",
            "\"audio/opus\" = \"\"",
            "4,3,4",
            "AUDIO preload=auto [object HTMLAudioElement] true",
            "{\"readyState\":0,\"networkState\":0,\"paused\":true,\"volume\":1,\"muted\":false,\"preload\":\"metadata\",\"playbackRate\":1,\"currentTime\":0,\"duration\":null,\"seeking\":false,\"ended\":false,\"error\":null,\"buffered\":0,\"preservesPitch\":true,\"crossOrigin\":null}",
            "IndexSizeError: Failed to execute 'start' on 'TimeRanges': The index provided (0) is greater than or equal to the maximum bound (0).",
        ]
    );
}

#[test]
fn a_graph_hears_silence_of_another_origins_element_without_cors() {
    let _turn = audio_turn();
    let capture = std::sync::Arc::new(Capture::paced());
    let _sink = Sink::set(capture.clone());
    let other = TestServer::plain(|request| ranged(request, "audio/webm", CLIP_AUDIO));
    let page = server(&format!(
        r#"
const wait = (m, e) => new Promise(r => m.addEventListener(e, r, {{once: true}}));
log('ready');
addEventListener('keydown', async () => {{
  const a = new Audio('{}');
  const ctx = new AudioContext();
  ctx.createMediaElementSource(a).connect(ctx.destination);
  await a.play();
  await wait(a, 'ended');
  log('done');
}}, {{ once: true }});
"#,
        other.url("/clip-audio.webm")
    ));
    let lines = run(&page, true);
    assert!(lines.is_empty(), "{lines:?}");
    let samples = capture.samples.lock().unwrap().clone();
    assert!(samples.len() > 48_000 * 2, "it played: {}", samples.len());
    let loudest = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert_eq!(loudest, 0.0);
}

#[test]
fn a_parsed_element_loads_and_sources_passed_over_say_so() {
    let page = TestServer::plain(move |request| match request.target.as_str() {
        "/page.html" => html(
            "<audio id=parsed src=/media/tone.wav></audio>\
             <video id=sources><source type=video/quicktime src=/media/x.mov><source><source src=/media/tone.wav type=audio/wav></video>\
             <script>const log = (s) => console.log(s);\n\
             const a = document.getElementById('parsed');\n\
             const v = document.getElementById('sources');\n\
             for (const s of v.querySelectorAll('source')) s.onerror = () => log('error at ' + (s.getAttribute('src') || 'none'));\n\
             log('ready');\n\
             a.addEventListener('canplaythrough', () => { log('parsed ' + a.duration + ' ' + v.currentSrc.replace(location.origin, '')); log('done'); });\n\
             </script>",
        ),
        "/media/tone.wav" => ranged(request, "audio/wav", TONE),
        _ => Reply::with_fields(404, &[], b""),
    });
    let lines = run(&page, false);
    assert_eq!(
        lines,
        [
            "error at /media/x.mov",
            "error at none",
            "parsed 1 /media/tone.wav"
        ]
    );
}

#[test]
fn media_elements_lay_out_as_the_reference_does() {
    let page = TestServer::plain(move |request| match request.target.as_str() {
        "/page.html" => html(
            "<body style=\"margin:0\">\
             <video id=v1></video><video id=v2 controls></video><audio id=a1 controls></audio><audio id=a2></audio>\
             <video id=v3 poster=\"/media/none.png\"></video><video id=v4 src=\"/media/clip.webm\" controls></video>\
             <script>const log = (s) => console.log(s);\n\
             log('ready');\n\
             const v4 = document.getElementById('v4');\n\
             v4.addEventListener('loadedmetadata', () => { log('a2 ' + getComputedStyle(document.getElementById('a2')).display); log('done'); });\n\
             </script>",
        ),
        "/media/clip.webm" => ranged(request, "video/webm", CLIP),
        _ => Reply::with_fields(404, &[], b""),
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(page.url("/page.html"));
    let lines = tab.run_until(|lines| lines.iter().any(|l| l == "done"));
    assert!(lines.iter().any(|l| l == "a2 none"), "{lines:?}");
    // The reference's boxes (probe p4).
    tab.engine.frame(800, 400);
    let nodes: Vec<_> = ["v1", "v2", "a1", "a2", "v3", "v4"]
        .iter()
        .map(|id| (*id, by_id(&tab.engine, id)))
        .collect();
    let layout = tab.engine.layout(800.0);
    let size = |id: &str| {
        let node = nodes.iter().find(|(n, _)| *n == id).expect("an id").1;
        layout
            .placed()
            .find(|p| p.fragment.node == node)
            .map(|p| (p.fragment.w, p.fragment.h))
    };
    assert_eq!(size("v1"), Some((300.0, 150.0)));
    assert_eq!(size("v2"), Some((300.0, 150.0)));
    assert_eq!(size("a1"), Some((300.0, 54.0)));
    assert_eq!(size("a2"), None);
    assert_eq!(size("v3"), Some((300.0, 150.0)));
    assert_eq!(size("v4"), Some((160.0, 120.0)));
}

#[test]
fn a_canvas_draws_a_videos_picture_at_its_time() {
    let page = server(
        r#"
const wait = (m, e) => new Promise(r => m.addEventListener(e, r, {once: true}));
function colour(v) {
  const c = document.createElement('canvas'); c.width = v.videoWidth; c.height = v.videoHeight;
  const g = c.getContext('2d'); g.drawImage(v, 0, 0);
  const d = g.getImageData(v.videoWidth >> 1, v.videoHeight >> 1, 1, 1).data;
  return [d[0], d[1], d[2]].join(',');
}
log('ready');
(async () => {
  const v = document.createElement('video'); document.body.appendChild(v);
  const blank = document.createElement('canvas').getContext('2d');
  blank.drawImage(v, 0, 0);
  log('nothing yet ' + blank.getImageData(0, 0, 1, 1).data.join(','));
  await createImageBitmap(v).then(() => log('?'), e => log(e.name + ': ' + e.message));
  v.src = '/media/clip.webm';
  await wait(v, 'canplaythrough');
  log('frame@0 ' + colour(v));
  v.currentTime = 1.5; await wait(v, 'seeked');
  log('frame@1.5 ' + colour(v));
  v.currentTime = 0.5; await wait(v, 'seeked');
  log('frame@0.5 ' + colour(v));
  const bitmap = await createImageBitmap(v);
  log('bitmap ' + bitmap.width + 'x' + bitmap.height);
  log('done');
})().catch(e => log('failed ' + e));
"#,
    );
    let lines = run(&page, false);
    // The reference's: 255,1,0 at 0 and 0.5; 0,255,1 at 1.5.
    let near = |line: &str, prefix: &str, want: [i32; 3]| {
        let rest = line
            .strip_prefix(prefix)
            .unwrap_or_else(|| panic!("{line}"));
        let got: Vec<i32> = rest.split(',').map(|v| v.parse().unwrap()).collect();
        assert!(
            got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 3),
            "{line}"
        );
    };
    assert_eq!(lines.len(), 6, "{lines:#?}");
    assert_eq!(lines[0], "nothing yet 0,0,0,0");
    assert_eq!(
        lines[1],
        "InvalidStateError: Failed to execute 'createImageBitmap' on 'Window': The provided element has not retrieved data."
    );
    near(&lines[2], "frame@0 ", [255, 1, 0]);
    near(&lines[3], "frame@1.5 ", [0, 255, 1]);
    near(&lines[4], "frame@0.5 ", [255, 1, 0]);
    assert_eq!(lines[5], "bitmap 160x120");
}

#[test]
fn a_video_shows_its_picture_and_its_controls_answer_the_pointer() {
    let _turn = audio_turn();
    let page = server(
        r#"
document.body.style.margin = '0';
const v = document.createElement('video');
v.style.width = '320px';
v.controls = true;
v.src = '/media/clip.webm';
document.body.appendChild(v);
log('ready');
v.addEventListener('canplaythrough', () => log('loaded'), { once: true });
for (const e of ['play', 'pause', 'volumechange', 'seeked']) v.addEventListener(e, () => log(e + ' paused=' + v.paused + ' muted=' + v.muted + ' t>1=' + (v.currentTime > 1)));
"#,
    );
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(page.url("/page.html"));
    tab.run_until(|lines| lines.iter().any(|l| l == "loaded"));
    // The picture at 0, red, fitted in its 320 by 240 box (under the
    // page's 100px `div`); the bar over its bottom darker.
    let pixels = tab.engine.frame(400, 400);
    let at = |x: usize, y: usize| {
        let i = (y * 400 + x) * 4;
        [pixels[i], pixels[i + 1], pixels[i + 2]]
    };
    let red = at(160, 200);
    assert!(red[0] > 240 && red[1] < 20 && red[2] < 20, "{red:?}");
    let bar = at(200, 336);
    assert!(
        bar[0] < 160 && bar[0] > 60,
        "the bar over the picture: {bar:?}"
    );
    // The play button plays (the press is the user's activation).
    tab.engine
        .pointer_down(20.0, 324.0, crate::Button::Primary, 1);
    tab.engine
        .pointer_up(20.0, 324.0, crate::Button::Primary, 1);
    let lines = tab.run_until(|lines| lines.iter().any(|l| l.starts_with("play")));
    assert_eq!(lines, ["play paused=false muted=false t>1=false"]);
    // The mute button.
    tab.engine.frame(400, 400);
    tab.engine
        .pointer_down(300.0, 324.0, crate::Button::Primary, 1);
    tab.engine
        .pointer_up(300.0, 324.0, crate::Button::Primary, 1);
    let lines = tab.run_until(|lines| lines.iter().any(|l| l.starts_with("volumechange")));
    assert_eq!(lines, ["volumechange paused=false muted=true t>1=false"]);
    // Three quarters along the timeline: a seek past a second.
    tab.engine.frame(400, 400);
    tab.engine
        .pointer_down(236.0, 324.0, crate::Button::Primary, 1);
    tab.engine
        .pointer_up(236.0, 324.0, crate::Button::Primary, 1);
    let lines = tab.run_until(|lines| lines.iter().any(|l| l.starts_with("seeked")));
    assert_eq!(lines, ["seeked paused=false muted=true t>1=true"]);
    // The play button again pauses.
    tab.engine.frame(400, 400);
    tab.engine
        .pointer_down(20.0, 324.0, crate::Button::Primary, 1);
    tab.engine
        .pointer_up(20.0, 324.0, crate::Button::Primary, 1);
    let lines = tab.run_until(|lines| lines.iter().any(|l| l.starts_with("pause")));
    assert_eq!(lines, ["pause paused=true muted=true t>1=true"]);
}

#[test]
fn a_video_shows_its_poster_until_it_has_a_picture() {
    const POSTER: &[u8] = include_bytes!("../../benches/pages/img/icon0.png");
    let page = TestServer::plain(move |request| match request.target.as_str() {
        "/page.html" => html(
            "<body style=margin:0><video id=v poster=/poster.png></video><script>console.log('ready')</script>",
        ),
        "/poster.png" => Reply::ok("image/png", POSTER),
        _ => Reply::with_fields(404, &[], b""),
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(page.url("/page.html"));
    tab.run_until(|lines| lines.iter().any(|l| l == "ready"));
    tab.wait_until(|engine| *engine.load_state() == crate::LoadState::Loaded);
    // Its natural size is the poster's, 24 by 24; it is drawn there.
    tab.engine.frame(100, 100);
    let node = by_id(&tab.engine, "v");
    let layout = tab.engine.layout(100.0);
    let size = layout
        .placed()
        .find(|p| p.fragment.node == node)
        .map(|p| (p.fragment.w, p.fragment.h));
    assert_eq!(size, Some((24.0, 24.0)));
    // Its pixels decoded once it is first drawn.
    let drawn = (0..200).any(|_| {
        tab.engine.poll_network();
        let pixels = tab.engine.frame(100, 100);
        let any = (0..24 * 24).any(|i| {
            let (x, y) = (i % 24, i / 24);
            let at = (y * 100 + x) * 4;
            pixels[at..at + 3] != [255, 255, 255]
        });
        if !any {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        any
    });
    assert!(drawn, "the poster is drawn");
}

#[test]
fn a_focused_media_elements_keys_play_pause_and_skip() {
    let _turn = audio_turn();
    let page = server(
        r#"
const v = document.createElement('video');
v.controls = true;
v.src = '/media/clip.webm';
document.body.appendChild(v);
log('ready');
v.addEventListener('canplaythrough', () => { v.focus(); log('focused ' + (document.activeElement === v)); }, { once: true });
for (const e of ['play', 'pause', 'seeked']) v.addEventListener(e, () => log(e + ' t=' + v.currentTime.toFixed(1)));
"#,
    );
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(page.url("/page.html"));
    let lines = tab.run_until(|lines| lines.iter().any(|l| l.starts_with("focused")));
    assert_eq!(lines.last().unwrap(), "focused true");
    tab.engine
        .key_down(&crate::Key::new(" ", "Space").with_text(" "));
    let lines = tab.run_until(|lines| lines.iter().any(|l| l.starts_with("play")));
    assert_eq!(lines, ["play t=0.0"]);
    tab.engine
        .key_down(&crate::Key::new("ArrowRight", "ArrowRight"));
    let lines = tab.run_until(|lines| lines.iter().any(|l| l.starts_with("pause")));
    // Five seconds on: the end of a clip of 2.1, where it stops.
    assert_eq!(lines, ["seeked t=2.1", "pause t=2.1"]);
    tab.engine
        .key_down(&crate::Key::new("ArrowLeft", "ArrowLeft"));
    let lines = tab.run_until(|lines| lines.iter().any(|l| l.starts_with("seeked")));
    assert_eq!(lines, ["seeked t=0.0"]);
    tab.engine
        .key_down(&crate::Key::new(" ", "Space").with_text(" "));
    let lines = tab.run_until(|lines| lines.iter().any(|l| l.starts_with("play")));
    assert_eq!(lines, ["play t=0.0"]);
}

#[test]
fn a_canvas_drawing_another_origins_video_is_tainted() {
    let other = TestServer::plain(|request| ranged(request, "video/webm", CLIP_VIDEO));
    let page = server(&format!(
        r#"
const wait = (m, e) => new Promise(r => m.addEventListener(e, r, {{once: true}}));
log('ready');
(async () => {{
  for (const cors of [false, true]) {{
    const v = document.createElement('video');
    if (cors) v.crossOrigin = 'anonymous';
    v.src = '{}';
    const loaded = await Promise.race([wait(v, 'canplaythrough').then(() => true), wait(v, 'error').then(() => false)]);
    if (!loaded) {{ log('cors ' + cors + ': error ' + v.error.code); continue; }}
    const g = document.createElement('canvas').getContext('2d');
    g.drawImage(v, 0, 0);
    try {{ g.getImageData(0, 0, 1, 1); log('cors ' + cors + ': read'); }} catch (e) {{ log('cors ' + cors + ': ' + e.name); }}
  }}
  log('done');
}})().catch(e => log('failed ' + e));
"#,
        other.url("/clip-video.webm")
    ));
    let lines = run(&page, false);
    // Without CORS, tainted; with it, refused by a server that allows
    // none.
    assert_eq!(lines, ["cors false: SecurityError", "cors true: error 4"]);
}

#[test]
fn a_videos_box_takes_its_pictures_size_on_a_page_with_no_script() {
    let page = TestServer::plain(move |request| match request.target.as_str() {
        "/page.html" => html("<body style=margin:0><video id=v src=/media/clip.webm></video>"),
        "/media/clip.webm" => ranged(request, "video/webm", CLIP),
        _ => Reply::with_fields(404, &[], b""),
    });
    let mut tab = Tab::new();
    tab.engine.navigate(page.url("/page.html"));
    tab.wait_until(|engine| *engine.load_state() == crate::LoadState::Loaded);
    let node = by_id(&tab.engine, "v");
    let mut size = None;
    for _ in 0..500 {
        tab.engine.poll_network();
        tab.engine.run_tasks();
        tab.engine.frame(400, 300);
        let layout = tab.engine.layout(400.0);
        size = layout
            .placed()
            .find(|p| p.fragment.node == node)
            .map(|p| (p.fragment.w, p.fragment.h));
        if size == Some((160.0, 120.0)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(size, Some((160.0, 120.0)));
}
