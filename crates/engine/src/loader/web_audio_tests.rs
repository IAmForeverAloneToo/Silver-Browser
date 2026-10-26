//! Web Audio in pages (M40 step 3): what an `OfflineAudioContext`
//! renders, as the reference browser rendered the same graphs (scratchpad
//! `m40/pages/p5`, Chrome 155), its errors in the reference browser's
//! words, `decodeAudioData()`, the events a context fires, and an
//! `AudioContext` that waits for the user's activation.

use net::test_server::{Reply, TestServer};

use crate::Button;
use crate::testing::*;

const TONE: &[u8] = include_bytes!("../../../media/fixtures/tone.wav");

fn server(script: &str) -> TestServer {
    let body = format!(
        "<style>body {{ margin: 0 }} div {{ width: 100px; height: 100px }}</style><div></div>\
         <script>const log = (s) => console.log(s);\n{script}</script>"
    );
    serve(move |path| match path {
        p if p.ends_with(".html") => Some(html(&body)),
        "/tone.wav" => Some(Reply::ok("audio/wav", TONE)),
        _ => None,
    })
}

fn open(server: &TestServer) -> Tab {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    tab
}

fn until_done(tab: &mut Tab) -> Vec<String> {
    tab.run_until(|lines| lines.iter().any(|l| l == "done" || l.starts_with("failed")))
        .into_iter()
        .filter(|l| l != "done")
        .collect()
}

/// Whether `got` is `wanted`, its numbers within `tolerance`.
fn close_to(got: &str, wanted: &str, tolerance: f64) -> bool {
    let (g, w): (Vec<&str>, Vec<&str>) = (got.split(' ').collect(), wanted.split(' ').collect());
    g.len() == w.len()
        && g.iter()
            .zip(&w)
            .all(|(g, w)| match (g.parse::<f64>(), w.parse::<f64>()) {
                (Ok(a), Ok(b)) => (a - b).abs() <= tolerance,
                _ => g == w,
            })
}

const RENDERS: &str = r#"
const R = 48000;
const pick = (a, ix) => ix.map(i => a[i].toFixed(6)).join(' ');
async function render(n, frames, build, ch = 1) {
  const c = new OfflineAudioContext(ch, frames, R); build(c); return c.startRendering();
}
(async () => {
  let b = await render('osc', 2048, c => { const o = c.createOscillator(); o.frequency.value = 440; o.connect(c.destination); o.start(); });
  log('osc sine ' + pick(b.getChannelData(0), [0, 1, 2, 3, 100, 1000, 2047]));
  for (const t of ['square', 'sawtooth', 'triangle']) {
    b = await render('osc', 2048, c => { const o = c.createOscillator(); o.type = t; o.frequency.value = 440; o.connect(c.destination); o.start(); });
    log('osc ' + t + ' ' + pick(b.getChannelData(0), [0, 1, 2, 50, 100, 1000]));
  }
  b = await render('ramp', 4800, c => { const s = c.createConstantSource(); const g = c.createGain(); s.connect(g); g.connect(c.destination);
    g.gain.setValueAtTime(0, 0); g.gain.linearRampToValueAtTime(1, 0.02); g.gain.exponentialRampToValueAtTime(0.01, 0.06); g.gain.setTargetAtTime(0.5, 0.07, 0.005); s.start(); });
  log('automation ' + pick(b.getChannelData(0), [0, 1, 480, 959, 960, 1440, 2000, 2879, 3360, 3600, 4000, 4799]));
  b = await render('biquad', 64, c => { const buf = c.createBuffer(1, 64, R); buf.getChannelData(0)[0] = 1; const s = c.createBufferSource(); s.buffer = buf;
    const f = c.createBiquadFilter(); f.type = 'lowpass'; f.frequency.value = 1000; f.Q.value = 1; s.connect(f); f.connect(c.destination); s.start(); });
  log('biquad lowpass impulse ' + pick(b.getChannelData(0), [0, 1, 2, 3, 4, 10, 63]));
  for (const t of ['highpass', 'bandpass', 'notch', 'peaking', 'lowshelf', 'highshelf', 'allpass']) {
    b = await render('biquad', 64, c => { const buf = c.createBuffer(1, 64, R); buf.getChannelData(0)[0] = 1; const s = c.createBufferSource(); s.buffer = buf;
      const f = c.createBiquadFilter(); f.type = t; f.frequency.value = 1000; f.Q.value = 1; f.gain.value = 6; s.connect(f); f.connect(c.destination); s.start(); });
    log('biquad ' + t + ' ' + pick(b.getChannelData(0), [0, 1, 2, 3, 10]));
  }
  b = await render('delay', 2048, c => { const buf = c.createBuffer(1, 4, R); buf.getChannelData(0).set([1, 0.5, 0.25, 0.125]); const s = c.createBufferSource(); s.buffer = buf;
    const d = c.createDelay(); d.delayTime.value = 0.01; s.connect(d); d.connect(c.destination); s.start(); });
  log('delay ' + pick(b.getChannelData(0), [0, 479, 480, 481, 482, 483, 484]));
  b = await render('pan', 128, c => { const s = c.createConstantSource(); const p = c.createStereoPanner(); p.pan.value = 0.5; s.connect(p); p.connect(c.destination); s.start(); }, 2);
  log('stereo pan 0.5 ' + pick(b.getChannelData(0), [0]) + ' / ' + pick(b.getChannelData(1), [0]));
  b = await render('upmix', 128, c => { const s = c.createConstantSource(); s.offset.value = 0.5; s.connect(c.destination); s.start(); }, 2);
  log('mono to stereo ' + pick(b.getChannelData(0), [0]) + ' / ' + pick(b.getChannelData(1), [0]));
  b = await render('rate', 32, c => { const buf = c.createBuffer(1, 16, R); buf.getChannelData(0).set([...Array(16).keys()]); const s = c.createBufferSource(); s.buffer = buf; s.playbackRate.value = 0.5; s.connect(c.destination); s.start(); });
  log('playbackRate 0.5 ' + pick(b.getChannelData(0), [0, 1, 2, 3, 4, 5, 29, 30, 31]));
  b = await render('loop', 16, c => { const buf = c.createBuffer(1, 4, R); buf.getChannelData(0).set([1, 2, 3, 4]); const s = c.createBufferSource(); s.buffer = buf; s.loop = true; s.loopStart = 1 / R; s.loopEnd = 3 / R; s.connect(c.destination); s.start(); });
  log('loop ' + pick(b.getChannelData(0), [0, 1, 2, 3, 4, 5, 6, 7]));
  b = await render('shaper', 4, c => { const buf = c.createBuffer(1, 4, R); buf.getChannelData(0).set([-1, -0.5, 0.25, 1]); const s = c.createBufferSource(); s.buffer = buf; const w = c.createWaveShaper(); w.curve = new Float32Array([0, 0.5, 1]); s.connect(w); w.connect(c.destination); s.start(); });
  log('waveshaper ' + pick(b.getChannelData(0), [0, 1, 2, 3]));
  b = await render('comp', 4800, c => { const s = c.createConstantSource(); s.offset.value = 0.9; const k = c.createDynamicsCompressor(); s.connect(k); k.connect(c.destination); s.start(); });
  log('compressor const 0.9 ' + pick(b.getChannelData(0), [0, 100, 1000, 4799]));
  b = await render('conv', 8, c => { const buf = c.createBuffer(1, 4, R); buf.getChannelData(0).set([1, 0, 0, 0]); const s = c.createBufferSource(); s.buffer = buf;
    const ir = c.createBuffer(1, 3, R); ir.getChannelData(0).set([0.5, 0.25, 0.125]); const v = c.createConvolver(); v.normalize = false; v.buffer = ir; s.connect(v); v.connect(c.destination); s.start(); });
  log('convolver ' + pick(b.getChannelData(0), [0, 1, 2, 3, 4]));
  b = await render('conv2', 8, c => { const buf = c.createBuffer(1, 4, R); buf.getChannelData(0).set([1, 0, 0, 0]); const s = c.createBufferSource(); s.buffer = buf;
    const ir = c.createBuffer(1, 3, R); ir.getChannelData(0).set([0.5, 0.25, 0.125]); const v = c.createConvolver(); v.buffer = ir; s.connect(v); v.connect(c.destination); s.start(); });
  log('convolver normalized ' + pick(b.getChannelData(0), [0, 1, 2, 3, 4]));
  b = await render('panner', 128, c => { const s = c.createConstantSource(); const p = c.createPanner(); p.panningModel = 'equalpower'; p.positionX.value = 1; s.connect(p); p.connect(c.destination); s.start(); }, 2);
  log('panner equalpower x=1 ' + pick(b.getChannelData(0), [0]) + ' / ' + pick(b.getChannelData(1), [0]));
  const ctx = new OfflineAudioContext(1, 1, R);
  for (const [what, f] of [['length 0', () => new OfflineAudioContext(1, 0, R)], ['rate 1000', () => new OfflineAudioContext(1, 1, 1000)],
      ['channels 0', () => new OfflineAudioContext(0, 1, R)], ['gain value NaN', () => { ctx.createGain().gain.value = NaN; }],
      ['buffer 0 ch', () => ctx.createBuffer(0, 1, R)], ['connect self ctx', () => ctx.createGain().connect(new OfflineAudioContext(1, 1, R).destination)],
      ['delay max 200', () => ctx.createDelay(200)], ['start twice', () => { const o = ctx.createOscillator(); o.start(); o.start(); }],
      ['exp ramp 0', () => ctx.createGain().gain.exponentialRampToValueAtTime(0, 1)], ['fftSize 100', () => { ctx.createAnalyser().fftSize = 100; }]]) {
    try { f(); log(what + ': ok'); } catch (e) { log(what + ': ' + e.name + ': ' + e.message); }
  }
  const dctx = new OfflineAudioContext(2, 1, 44100);
  const wav = await (await fetch('/tone.wav')).arrayBuffer();
  const d = await dctx.decodeAudioData(wav); log('decode 48k wav into 44.1k ctx: ' + d.sampleRate + ' ' + d.length + ' ' + d.getChannelData(0)[100].toFixed(6));
  await dctx.decodeAudioData(new ArrayBuffer(16)).then(() => log('?'), e => log('decode junk: ' + e.name + ': ' + e.message));
  log('done');
})().catch(e => log('failed ' + e + ' ' + e.stack));
"#;

/// Chrome's lines for `RENDERS` (scratchpad `m40/results.txt`).
const CHROME: &[&str] = &[
    "osc sine 0.000000 0.057564 0.114937 0.171929 -0.500000 0.866025 -0.996041",
    "osc square 0.000000 0.950091 0.848893 0.857234 -0.853281 0.855590",
    "osc sawtooth 0.000000 0.011840 0.036469 0.783591 -0.136411 0.287230",
    "osc triangle 0.000000 0.036557 0.073498 0.166884 -0.333278 0.667195",
    "automation 0.000000 0.001042 0.500000 0.998958 1.000000 0.316228 0.082540 0.010024 0.010000 0.319739 0.465953 0.498780",
    "biquad lowpass impulse 0.004042 0.015660 0.029790 0.041884 0.051972 0.075111 0.003328",
    "biquad highpass 0.940989 -0.118665 -0.118916 -0.117217 -0.070691",
    "biquad bandpass 0.061265 0.114039 0.097250 0.080956 -0.006618",
    "biquad notch 0.938735 -0.114039 -0.097250 -0.080956 0.006618",
    "biquad peaking 1.043953 0.083305 0.073866 0.064052 -0.000825",
    "biquad lowshelf 1.032562 0.065660 0.066281 0.066066 0.050297",
    "biquad highshelf 1.932341 -0.122876 -0.116224 -0.108358 -0.046430",
    "biquad allpass 0.877470 -0.228077 -0.194500 -0.161912 0.013236",
    "delay 0.000000 0.000000 1.000000 0.500000 0.250000 0.125000 0.000000",
    "stereo pan 0.5 0.382683 / 0.923880",
    "mono to stereo 0.500000 / 0.500000",
    "playbackRate 0.5 0.000000 0.500000 1.000000 1.500000 2.000000 2.500000 14.500000 15.000000 15.500000",
    "loop 1.000000 2.000000 3.000000 2.000000 3.000000 2.000000 3.000000 2.000000",
    "waveshaper 0.000000 0.250000 0.625000 1.000000",
    // The compressor's curve is its own (the design note's section 4):
    // only its shape is checked.
    "compressor const 0.9",
    "convolver 0.500000 0.250000 0.125000 0.000000 0.000000",
    "convolver normalized 0.001749 0.000874 0.000437 0.000000 0.000000",
    "panner equalpower x=1 0.000000 / 1.000000",
    "length 0: NotSupportedError: Failed to construct 'OfflineAudioContext': The number of frames provided (0) is less than the minimum bound (1).",
    "rate 1000: NotSupportedError: Failed to construct 'OfflineAudioContext': The sampleRate provided (1000) is outside the range [3000, 768000].",
    "channels 0: NotSupportedError: Failed to construct 'OfflineAudioContext': The number of channels provided (0) is outside the range [1, 32].",
    "gain value NaN: TypeError: Failed to set the 'value' property on 'AudioParam': The provided float value is non-finite.",
    "buffer 0 ch: NotSupportedError: Failed to execute 'createBuffer' on 'BaseAudioContext': The number of channels provided (0) is outside the range [1, 32].",
    "connect self ctx: InvalidAccessError: Failed to execute 'connect' on 'AudioNode': cannot connect to an AudioNode belonging to a different audio context.",
    "delay max 200: NotSupportedError: Failed to execute 'createDelay' on 'BaseAudioContext': The max delay time provided (200) is outside the range (0, 180).",
    "start twice: InvalidStateError: Failed to execute 'start' on 'AudioScheduledSourceNode': cannot call start more than once.",
    "exp ramp 0: RangeError: Failed to execute 'exponentialRampToValueAtTime' on 'AudioParam': The float target value provided (0) should not be in the range (-1.40130e-45, 1.40130e-45).",
    "fftSize 100: IndexSizeError: Failed to set the 'fftSize' property on 'AnalyserNode': The value provided (100) is not a power of two.",
    "decode 48k wav into 44.1k ctx: 44100 44099 -0.007150",
    "decode junk: EncodingError: Unable to decode audio data",
];

#[test]
fn an_offline_context_renders_what_the_reference_browser_rendered() {
    let server = server(RENDERS);
    let mut tab = open(&server);
    let lines = until_done(&mut tab);
    assert_eq!(lines.len(), CHROME.len(), "{lines:#?}");
    for (got, wanted) in lines.iter().zip(CHROME) {
        if *wanted == "compressor const 0.9" {
            // Silent at first (its look-ahead), then reduced below 0.9.
            let v: Vec<f64> = got[wanted.len()..]
                .split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect();
            assert!(v[0] == 0.0 && v[3] > 0.2 && v[3] < 0.9, "{got}");
            continue;
        }
        if wanted.starts_with("decode 48k") {
            // Chrome's resampling ratio, a float, leaves its buffer a
            // frame short; its kernel is not this one.
            let v = |l: &str| -> Vec<f64> {
                l.rsplit(": ")
                    .next()
                    .unwrap()
                    .split(' ')
                    .map(|v| v.parse().unwrap())
                    .collect()
            };
            let (g, w) = (v(got), v(wanted));
            assert!(
                g[0] == w[0] && (g[1] - w[1]).abs() <= 1.0 && (g[2] - w[2]).abs() <= 5e-5,
                "{got}"
            );
            continue;
        }
        // Rendering in f32 leaves the last digit of six to rounding.
        assert!(close_to(got, wanted, 2e-6), "got {got}\nwanted {wanted}");
    }
}

#[test]
fn an_offline_context_suspends_completes_and_tells_its_states() {
    let server = server(
        r#"
(async () => {
  const c = new OfflineAudioContext({ numberOfChannels: 1, length: 512, sampleRate: 48000 });
  c.onstatechange = () => log('statechange ' + c.state);
  c.oncomplete = (e) => log('complete ' + e.renderedBuffer.length + ' ' + (e instanceof OfflineAudioCompletionEvent));
  const s = c.createBufferSource();
  const buf = c.createBuffer(1, 10, 48000);
  buf.getChannelData(0).fill(0.25);
  s.buffer = buf;
  s.onended = () => log('ended');
  s.connect(c.destination);
  s.start();
  log('state ' + c.state + ' length ' + c.length);
  c.suspend(256 / 48000).then(() => {
    log('suspended at ' + Math.round(c.currentTime * 48000) + ' ' + c.state);
    c.resume();
  });
  try { await c.suspend(256 / 48000); } catch (e) { log(e.name + ': ' + e.message); }
  try { await c.suspend(1); } catch (e) { log(e.name + ': ' + e.message); }
  const b = await c.startRendering();
  log('rendered ' + b.getChannelData(0)[0] + ' ' + b.getChannelData(0)[9] + ' ' + b.getChannelData(0)[10] + ' ' + c.state);
  try { await c.startRendering(); } catch (e) { log(e.name + ': ' + e.message); }
  log('done');
})().catch(e => log('failed ' + e + ' ' + e.stack));
"#,
    );
    let mut tab = open(&server);
    let lines = until_done(&mut tab);
    assert_eq!(
        lines,
        [
            "state suspended length 512",
            "InvalidStateError: Failed to execute 'suspend' on 'OfflineAudioContext': cannot schedule more than one suspend at frame 256 (0.005333333333333333 seconds)",
            "InvalidStateError: Failed to execute 'suspend' on 'OfflineAudioContext': cannot schedule a suspend at frame 48000 (1 seconds) because it is greater than or equal to the total render duration of 512 frames",
            "statechange running",
            "ended",
            "statechange suspended",
            "suspended at 256 suspended",
            "statechange running",
            "statechange closed",
            "complete 512 true",
            "rendered 0.25 0.25 0 closed",
            "InvalidStateError: Failed to execute 'startRendering' on 'OfflineAudioContext': cannot call startRendering more than once",
        ]
    );
}

#[test]
fn nodes_check_what_a_page_gives_them_in_the_reference_browsers_words() {
    let server = server(
        r#"
const c = new OfflineAudioContext(2, 128, 48000);
const t = (what, f) => { try { log(what + ': ' + f()); } catch (e) { log(what + ': ' + e.name + ': ' + e.message); } };
t('interfaces', () => [AudioNode, AudioParam, GainNode, OscillatorNode, PeriodicWave, AudioBuffer, AudioListener,
  AnalyserNode, ConvolverNode, IIRFilterNode, PannerNode, StereoPannerNode, WaveShaperNode, DynamicsCompressorNode,
  ChannelSplitterNode, ChannelMergerNode, DelayNode, BiquadFilterNode, ConstantSourceNode, AudioBufferSourceNode,
  AudioDestinationNode, AudioScheduledSourceNode, BaseAudioContext, AudioContext].every(f => typeof f === 'function'));
t('illegal', () => new AudioNode());
t('gain', () => { const g = new GainNode(c, { gain: 0.5 }); return [g.gain.value, g.gain.defaultValue, g.gain.minValue > -4e38, g.numberOfInputs, g.channelCount, g.channelCountMode, g.channelInterpretation, g.context === c].join(); });
t('param chain', () => { const g = c.createGain(); return g.gain.setValueAtTime(1, 0) === g.gain; });
t('destination', () => [c.destination.maxChannelCount, c.destination.channelCount, c.destination.numberOfOutputs].join());
t('dest count 3', () => { c.destination.channelCount = 3; });
t('panner count 3', () => { c.createStereoPanner().channelCount = 3; });
t('panner max', () => { c.createPanner().channelCountMode = 'max'; });
t('splitter count', () => { c.createChannelSplitter(4).channelCount = 2; });
t('splitter 33', () => c.createChannelSplitter(33));
t('merger shape', () => { const m = c.createChannelMerger(3); return [m.numberOfInputs, m.numberOfOutputs, m.channelCount].join(); });
t('connect output 1', () => c.createGain().connect(c.destination, 1));
t('connect returns', () => { const g = c.createGain(); return g.connect(c.destination) === c.destination; });
t('connect param', () => c.createOscillator().connect(c.createGain().gain));
t('disconnect unconnected', () => { const a = c.createGain(), b = c.createGain(); a.disconnect(b); });
t('disconnect connected', () => { const a = c.createGain(), b = c.createGain(); a.connect(b); a.disconnect(b); return 'ok'; });
t('stop before start', () => c.createOscillator().stop());
t('osc custom', () => { c.createOscillator().type = 'custom'; });
t('osc type bogus', () => { const o = c.createOscillator(); o.type = 'bogus'; return o.type; });
t('wave', () => { const o = c.createOscillator(); o.setPeriodicWave(c.createPeriodicWave(new Float32Array([0, 1]), new Float32Array([0, 0]))); return o.type; });
t('wave lengths', () => c.createPeriodicWave(new Float32Array(3), new Float32Array(4)));
t('buffer channel', () => c.createBuffer(1, 4, 48000).getChannelData(1));
t('buffer shape', () => { const b = new AudioBuffer({ length: 480, sampleRate: 48000, numberOfChannels: 2 }); return [b.length, b.duration, b.numberOfChannels, b.sampleRate].join(); });
t('buffer no rate', () => new AudioBuffer({ length: 4 }));
t('buffer twice', () => { const s = c.createBufferSource(); s.buffer = c.createBuffer(1, 1, 48000); s.buffer = c.createBuffer(1, 1, 48000); });
t('convolver 3 ch', () => { c.createConvolver().buffer = c.createBuffer(3, 1, 48000); });
t('convolver rate', () => { c.createConvolver().buffer = c.createBuffer(1, 1, 44100); });
t('curve short', () => { c.createWaveShaper().curve = new Float32Array(1); });
t('analyser', () => { const a = c.createAnalyser(); return [a.fftSize, a.frequencyBinCount, a.minDecibels, a.maxDecibels, a.smoothingTimeConstant].join(); });
t('analyser min', () => { c.createAnalyser().minDecibels = -20; });
t('iir zero', () => c.createIIRFilter([1], [0]));
t('iir long', () => c.createIIRFilter(new Array(21).fill(1), [1]));
t('curve time', () => c.createGain().gain.setValueCurveAtTime(new Float32Array([1, 2]), -1, 1));
t('target NaN', () => c.createGain().gain.setTargetAtTime(1, 0, NaN));
t('k-rate fixed', () => { c.createBufferSource().playbackRate.automationRate = 'a-rate'; });
t('rate changes', () => { const g = c.createGain(); g.gain.automationRate = 'k-rate'; return g.gain.automationRate; });
t('response', () => { const f = c.createBiquadFilter(); const m = new Float32Array(1), p = new Float32Array(1); f.getFrequencyResponse(new Float32Array([0]), m, p); return m[0].toFixed(4) + ' ' + p[0].toFixed(4); });
t('response lengths', () => c.createBiquadFilter().getFrequencyResponse(new Float32Array(2), new Float32Array(1), new Float32Array(2)));
t('listener', () => [c.listener.forwardZ.value, c.listener.upY.value].join());
t('huge offline', () => new OfflineAudioContext(32, 1e8, 48000));
t('many contexts', () => { for (let i = 0; i < 40; i++) new OfflineAudioContext(1, 1, 48000); });
log('done');
"#,
    );
    let mut tab = open(&server);
    let lines = until_done(&mut tab);
    assert_eq!(
        lines,
        [
            "interfaces: true",
            "illegal: TypeError: Failed to construct 'AudioNode': Illegal constructor",
            "gain: 0.5,1,true,1,2,max,speakers,true",
            "param chain: true",
            "destination: 2,2,1",
            "dest count 3: IndexSizeError: Failed to set the 'channelCount' property on 'AudioNode': The channel count provided (3) is outside the range [1, 2].",
            "panner count 3: NotSupportedError: Failed to set the 'channelCount' property on 'AudioNode': The channel count provided (3) is outside the range [1, 2].",
            "panner max: NotSupportedError: Failed to set the 'channelCountMode' property on 'AudioNode': 'max' is not allowed.",
            "splitter count: InvalidStateError: Failed to set the 'channelCount' property on 'AudioNode': Cannot change channel count of ChannelSplitterNode.",
            "splitter 33: IndexSizeError: Failed to execute 'createChannelSplitter' on 'BaseAudioContext': The number of outputs provided (33) is outside the range [1, 32].",
            "merger shape: 3,1,1",
            "connect output 1: IndexSizeError: Failed to execute 'connect' on 'AudioNode': output index (1) exceeds number of outputs (1).",
            "connect returns: true",
            "connect param: undefined",
            "disconnect unconnected: InvalidAccessError: Failed to execute 'disconnect' on 'AudioNode': the given destination is not connected.",
            "disconnect connected: ok",
            "stop before start: InvalidStateError: Failed to execute 'stop' on 'AudioScheduledSourceNode': cannot call stop without calling start first.",
            "osc custom: InvalidStateError: Failed to set the 'type' property on 'OscillatorNode': 'type' cannot be set directly to 'custom'.  Use setPeriodicWave() to create a custom Oscillator type.",
            "osc type bogus: sine",
            "wave: custom",
            "wave lengths: IndexSizeError: Failed to construct 'PeriodicWave': real and imag arrays must be the same length (3 vs 4).",
            "buffer channel: IndexSizeError: Failed to execute 'getChannelData' on 'AudioBuffer': channel index (1) exceeds number of channels (1)",
            "buffer shape: 480,0.01,2,48000",
            "buffer no rate: TypeError: Failed to construct 'AudioBuffer': Failed to read the 'sampleRate' property from 'AudioBufferOptions': Required member is undefined.",
            "buffer twice: InvalidStateError: Failed to set the 'buffer' property on 'AudioBufferSourceNode': Cannot set buffer to non-null after it has been already been set to a non-null buffer",
            "convolver 3 ch: NotSupportedError: Failed to set the 'buffer' property on 'ConvolverNode': The buffer must have 1, 2, or 4 channels, not 3",
            "convolver rate: NotSupportedError: Failed to set the 'buffer' property on 'ConvolverNode': The buffer sample rate 44100 does not match the context rate 48000 Hz.",
            "curve short: InvalidStateError: Failed to set the 'curve' property on 'WaveShaperNode': The curve length provided (1) is less than the minimum bound (2).",
            "analyser: 2048,1024,-100,-30,0.8",
            "analyser min: IndexSizeError: Failed to set the 'minDecibels' property on 'AnalyserNode': The minDecibels provided (-20) is greater than or equal to the maximum bound (-30).",
            "iir zero: InvalidStateError: Failed to construct 'IIRFilterNode': First feedback coefficient cannot be zero.",
            "iir long: NotSupportedError: Failed to construct 'IIRFilterNode': The feedforward array length provided (21) is outside the range [1, 20].",
            "curve time: RangeError: Failed to execute 'setValueCurveAtTime' on 'AudioParam': The start time provided (-1) is less than the minimum bound (0).",
            "target NaN: TypeError: Failed to execute 'setTargetAtTime' on 'AudioParam': The provided double value is non-finite.",
            "k-rate fixed: InvalidStateError: Failed to set the 'automationRate' property on 'AudioParam': automationRate cannot be changed",
            "rate changes: k-rate",
            "response: 1.0000 0.0000",
            "response lengths: InvalidAccessError: Failed to execute 'getFrequencyResponse' on 'BiquadFilterNode': magResponse length (1) is not equal to frequencyHz length (2).",
            "listener: -1,1",
            "huge offline: NotSupportedError: Failed to construct 'OfflineAudioContext': the rendered buffer would be too large",
            "many contexts: NotSupportedError: Failed to construct 'OfflineAudioContext': too many audio contexts",
        ]
    );
}

#[test]
fn decode_audio_data_calls_back_and_keeps_the_pages_buffer() {
    let server = server(
        r#"
(async () => {
  const c = new OfflineAudioContext(1, 1, 48000);
  const wav = await (await fetch('/tone.wav')).arrayBuffer();
  const size = wav.byteLength;
  const b = await c.decodeAudioData(wav, (d) => log('success ' + d.sampleRate + ' ' + d.numberOfChannels));
  log('decoded ' + b.length + ' ' + b.duration.toFixed(3) + ' kept ' + (wav.byteLength === size));
  await c.decodeAudioData(new ArrayBuffer(8), null, (e) => log('error callback ' + e.name)).catch(e => log('rejected ' + e.message));
  await c.decodeAudioData('nope').catch(e => log(e.name + ': ' + e.message));
  // A buffer decoded in a context plays there by the engine's copy
  // until the page changes it.
  const r = new OfflineAudioContext(2, 4, 48000);
  const mine = await r.decodeAudioData(wav);
  const s = r.createBufferSource();
  mine.getChannelData(0)[1] = 0.75;
  s.buffer = mine;
  s.connect(r.destination);
  s.start();
  const out = await r.startRendering();
  log('played ' + out.getChannelData(0)[1]);
  log('done');
})().catch(e => log('failed ' + e + ' ' + e.stack));
"#,
    );
    let mut tab = open(&server);
    let lines = until_done(&mut tab);
    assert_eq!(
        lines,
        [
            "success 48000 2",
            "decoded 48000 1.000 kept true",
            "error callback EncodingError",
            "rejected Unable to decode audio data",
            "TypeError: Failed to execute 'decodeAudioData' on 'BaseAudioContext': parameter 1 is not of type 'ArrayBuffer'.",
            "played 0.75",
        ]
    );
}

#[test]
fn an_audio_context_waits_for_the_users_activation_to_run() {
    let _turn = audio_turn();
    let first = server(
        r#"
const c = new AudioContext();
c.onstatechange = () => log('statechange ' + c.state);
log('made ' + c.state + ' ' + c.sampleRate + ' ' + c.baseLatency);
c.resume().then(() => log('resumed ' + c.state));
"#,
    );
    let mut tab = open(&first);
    let lines = tab.run_until(|lines| lines.iter().any(|l| l.starts_with("made")));
    assert_eq!(lines, ["made suspended 48000 0.01"]);
    tab.engine.pointer_down(50.0, 50.0, Button::Primary, 1);
    let lines = tab.run_until(|lines| lines.iter().any(|l| l.starts_with("resumed")));
    assert_eq!(lines, ["statechange running", "resumed running"]);
    // Running, its clock moves; suspended, closed, it tells each.
    let second = server(
        r#"
(async () => {
  const c = new AudioContext();
  log('made ' + c.state);
  await new Promise(r => setTimeout(r, 100));
  log('moved ' + (c.currentTime > 0));
  c.onstatechange = () => log('statechange ' + c.state);
  await c.suspend();
  log('after suspend ' + c.state);
  await c.close();
  log('after close ' + c.state);
  await c.close().catch(e => log(e.name + ': ' + e.message));
  await c.resume().catch(e => log(e.name + ': ' + e.message));
  log('done');
})().catch(e => log('failed ' + e + ' ' + e.stack));
"#,
    );
    // The tab was activated: a new document's context runs at once.
    tab.engine.navigate(second.url("/page.html"));
    let lines = until_done(&mut tab);
    assert_eq!(
        lines,
        [
            "made suspended",
            "moved true",
            "statechange suspended",
            "after suspend suspended",
            "statechange closed",
            "after close closed",
            "InvalidStateError: Failed to execute 'close' on 'AudioContext': Cannot close a context that is being closed or has already been closed.",
            "InvalidStateError: Failed to execute 'resume' on 'AudioContext': Cannot resume a closed AudioContext.",
        ]
    );
}

/// The zero crossings of the left channel of `samples`, interleaved
/// stereo.
fn crossings(samples: &[f32]) -> usize {
    let left: Vec<f32> = samples.iter().step_by(2).copied().collect();
    left.windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count()
}

/// The process's mixer writes to `sink` while this lives; then to a
/// sink that plays at the clock again.
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

/// A sink that says it played ten million frames.
struct Far;

impl crate::AudioSink for Far {
    fn write(&self, _: &[f32]) {}

    fn played(&self) -> u64 {
        10_000_000
    }
}

#[test]
fn an_audio_context_is_heard_at_the_outputs_rate() {
    let _turn = audio_turn();
    // A sink that said it played far more than the next one will have:
    // the mixer starts over with the new one.
    let _far = Sink::set(std::sync::Arc::new(Far));
    std::thread::sleep(std::time::Duration::from_millis(50));
    let capture = std::sync::Arc::new(Capture::default());
    let _sink = Sink::set(capture.clone());
    let page = server(
        r#"
const rate = Number(location.pathname.replace(/\D/g, ''));
if (rate) {
  const c = new AudioContext({ sampleRate: rate });
  const o = c.createOscillator();
  o.frequency.value = 440;
  const g = c.createGain();
  g.gain.value = 0.5;
  o.connect(g).connect(c.destination);
  o.start();
  // A mono destination: the mixer up-mixes it.
  if (rate === 44100) c.destination.channelCount = 1;
  log('state ' + c.state + ' ' + c.sampleRate);
  c.onstatechange = () => log('then ' + c.state);
} else {
  log('blank');
}
"#,
    );
    let mut tab = open(&page);
    tab.run_until(|lines| lines.iter().any(|l| l == "blank"));
    tab.engine.pointer_down(50.0, 50.0, Button::Primary, 1);
    for rate in [48000, 44100] {
        tab.engine.navigate(page.url(&format!("/tone{rate}.html")));
        let lines = tab.run_until(|lines| lines.iter().any(|l| l.starts_with("then")));
        // Running once its first task is told (WPT's
        // constructor-allowed-to-start.html), and heard from then.
        assert_eq!(
            lines[lines.len() - 2..],
            [format!("state suspended {rate}"), "then running".to_owned()]
        );
        capture.samples.lock().unwrap().clear();
        // A fifth of a second of sound, at the output's rate, with no gap
        // in it: a busy machine's mixer may run dry a moment (the gate's
        // run counted 189 crossings for a gap's).
        wait_for(|| capture.samples.lock().unwrap().len() >= 48000 * 2);
        let captured = capture.samples.lock().unwrap().clone();
        let samples = unbroken(&captured, 0.1).to_vec();
        // 440 Hz: 176 crossings in a fifth of a second.
        let n = crossings(&samples);
        assert!((173..=179).contains(&n), "{rate}: {n} crossings");
        let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((0.45..=0.5001).contains(&peak), "{rate}: peak {peak}");
        // Both channels: the mono oscillator up-mixed, by the graph or
        // the mixer.
        assert!(samples.chunks(2).all(|f| f[0] == f[1]), "{rate}");
    }
}

#[test]
fn the_runners_autoplay_starts_a_context_in_a_task_after_it_is_made() {
    // WPT's constructor-allowed-to-start.html, as the web-platform tests'
    // runner lets sound start without the user's activation (M40 step 9).
    let _turn = audio_turn();
    let page = server(
        r#"
const c = new AudioContext();
log('made ' + c.state);
c.onstatechange = () => { log('statechange ' + c.state); c.close().then(() => log('done')); };
"#,
    );
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.set_autoplay(true);
    tab.engine.navigate(page.url("/page.html"));
    let lines = until_done(&mut tab);
    assert_eq!(lines[..2], ["made suspended", "statechange running"]);
}

#[test]
fn options_and_floats_are_checked_as_the_reference_checks_them() {
    // WPT's ctor-*.html, pannernode-setposition-throws.html,
    // createPeriodicWaveInfiniteValuesThrows.html and audionode.html.
    let page = server(
        r#"
const c = new OfflineAudioContext(1, 128, 48000);
const t = (what, f) => { try { f(); log(what + ' ok'); } catch (e) { log(what + ' ' + e.name); } };
t('number options', () => new GainNode(c, 42));
t('bad mode', () => new GainNode(c, { channelCountMode: 'foobar' }));
t('bad interpretation', () => new GainNode(c, { channelInterpretation: 'foobar' }));
t('good options', () => new GainNode(c, { channelCountMode: 'explicit', channelInterpretation: 'discrete' }));
t('context options', () => new AudioContext(1));
const p = new PannerNode(c);
t('far position', () => p.setPosition(1, 0, 2 * 3.4028234663852886e38));
log('position ' + p.positionX.value + ' ' + p.positionZ.value);
t('far orientation', () => p.setOrientation(0, 2 * 3.4028234663852886e38, 0));
log('orientation ' + p.orientationX.value);
t('infinite wave', () => c.createPeriodicWave(new Float32Array([0, Infinity]), new Float32Array(2)));
log('done');
"#,
    );
    let mut tab = open(&page);
    assert_eq!(
        until_done(&mut tab),
        [
            "number options TypeError",
            "bad mode TypeError",
            "bad interpretation TypeError",
            "good options ok",
            "context options TypeError",
            "far position TypeError",
            "position 0 0",
            "far orientation TypeError",
            "orientation 1",
            "infinite wave TypeError",
        ]
    );
}
