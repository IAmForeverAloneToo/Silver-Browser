//! The graphs of probe p5, rendered here and held to the reference
//! browser's numbers.

use std::sync::Arc;

use crate::nodes::filters::BiquadType;
use crate::nodes::{self, Kind};
use crate::wave::Shape;
use crate::*;

const R: f32 = 48_000.0;

/// Renders `frames` frames of a graph of `channels` channels `build`
/// made; each channel's frames.
fn render(frames: usize, channels: usize, build: impl FnOnce(&mut Graph)) -> Vec<Vec<f32>> {
    let mut graph = Graph::new(R, channels);
    build(&mut graph);
    let mut out = vec![Vec::new(); channels];
    while out[0].len() < frames {
        let bus = graph.render();
        for (o, b) in out.iter_mut().zip(&bus) {
            o.extend_from_slice(b);
        }
    }
    out
}

fn check(got: &[f32], at: &[usize], want: &[f32], tolerance: f32, what: &str) {
    for (&i, &w) in at.iter().zip(want) {
        assert!(
            (got[i] - w).abs() <= tolerance,
            "{what}: frame {i} is {}, the reference's {w}",
            got[i]
        );
    }
}

fn started(graph: &mut Graph, id: NodeId) {
    match &mut graph.node_mut(id).unwrap().kind {
        Kind::Oscillator(o) => o.schedule.start = Some(0.0),
        Kind::Constant(s) => s.start = Some(0.0),
        Kind::BufferSource(b) => b.schedule.start = Some(0.0),
        _ => panic!("not a source"),
    }
}

fn buffer(values: &[f32]) -> Buffer {
    Arc::new(AudioData {
        rate: R,
        channels: vec![values.to_vec()],
    })
}

/// An impulse through `node`'s first input to the destination.
fn impulse_through(graph: &mut Graph, node: NodeId, length: usize) {
    let mut impulse = vec![0.0; length];
    impulse[0] = 1.0;
    let source = graph.add(nodes::buffer_source()).unwrap();
    if let Kind::BufferSource(b) = &mut graph.node_mut(source).unwrap().kind {
        b.buffer = Some(buffer(&impulse));
    }
    started(graph, source);
    graph.connect(source, 0, node, 0);
    graph.connect(node, 0, 0, 0);
}

#[test]
fn oscillators_give_the_reference_browsers_waves() {
    let at = [0, 1, 2, 3, 100, 1000, 2047];
    let out = render(2048, 1, |g| {
        let o = g.add(nodes::oscillator(R)).unwrap();
        started(g, o);
        g.connect(o, 0, 0, 0);
    });
    check(
        &out[0],
        &at,
        &[0.0, 0.057564, 0.114937, 0.171929, -0.5, 0.866025, -0.996041],
        2e-6,
        "sine",
    );
    let at = [0, 1, 2, 50, 100, 1000];
    for (shape, want) in [
        (
            Shape::Square,
            [0.0, 0.950091, 0.848893, 0.857234, -0.853281, 0.855590],
        ),
        (
            Shape::Sawtooth,
            [0.0, 0.011840, 0.036469, 0.783591, -0.136411, 0.287230],
        ),
        (
            Shape::Triangle,
            [0.0, 0.036557, 0.073498, 0.166884, -0.333278, 0.667195],
        ),
    ] {
        let out = render(2048, 1, |g| {
            let o = g.add(nodes::oscillator(R)).unwrap();
            if let Kind::Oscillator(osc) = &mut g.node_mut(o).unwrap().kind {
                osc.set_shape(shape);
            }
            started(g, o);
            g.connect(o, 0, 0, 0);
        });
        check(&out[0], &at, &want, 2e-5, &format!("{shape:?}"));
    }
}

#[test]
fn a_params_automation_reaches_the_graph_frame_by_frame() {
    let out = render(4800, 1, |g| {
        let s = g.add(nodes::constant()).unwrap();
        let gain = g.add(nodes::gain()).unwrap();
        let p = &mut g.node_mut(gain).unwrap().params[0];
        p.schedule(
            Event::SetValue {
                value: 0.0,
                time: 0.0,
            },
            0.0,
        )
        .unwrap();
        p.schedule(
            Event::LinearRamp {
                value: 1.0,
                time: 0.02,
            },
            0.0,
        )
        .unwrap();
        p.schedule(
            Event::ExponentialRamp {
                value: 0.01,
                time: 0.06,
            },
            0.0,
        )
        .unwrap();
        p.schedule(
            Event::SetTarget {
                target: 0.5,
                time: 0.07,
                constant: 0.005,
            },
            0.0,
        )
        .unwrap();
        started(g, s);
        g.connect(s, 0, gain, 0);
        g.connect(gain, 0, 0, 0);
    });
    check(
        &out[0],
        &[
            0, 1, 480, 959, 960, 1440, 2000, 2879, 3360, 3600, 4000, 4799,
        ],
        &[
            0.0, 0.001042, 0.5, 0.998958, 1.0, 0.316228, 0.082540, 0.010024, 0.01, 0.319739,
            0.465953, 0.498780,
        ],
        2e-6,
        "automation",
    );
}

#[test]
fn every_biquad_type_has_the_reference_browsers_impulse_response() {
    for (kind, want) in [
        (
            BiquadType::Lowpass,
            [0.004042, 0.015660, 0.029790, 0.041884, 0.075111],
        ),
        (
            BiquadType::Highpass,
            [0.940989, -0.118665, -0.118916, -0.117217, -0.070691],
        ),
        (
            BiquadType::Bandpass,
            [0.061265, 0.114039, 0.097250, 0.080956, -0.006618],
        ),
        (
            BiquadType::Notch,
            [0.938735, -0.114039, -0.097250, -0.080956, 0.006618],
        ),
        (
            BiquadType::Peaking,
            [1.043953, 0.083305, 0.073866, 0.064052, -0.000825],
        ),
        (
            BiquadType::Lowshelf,
            [1.032562, 0.065660, 0.066281, 0.066066, 0.050297],
        ),
        (
            BiquadType::Highshelf,
            [1.932341, -0.122876, -0.116224, -0.108358, -0.046430],
        ),
        (
            BiquadType::Allpass,
            [0.877470, -0.228077, -0.194500, -0.161912, 0.013236],
        ),
    ] {
        let out = render(64, 1, |g| {
            let f = g.add(nodes::biquad(R)).unwrap();
            let node = g.node_mut(f).unwrap();
            if let Kind::Biquad(b) = &mut node.kind {
                b.kind = kind;
            }
            node.params[0].value = 1000.0;
            node.params[2].value = 1.0;
            node.params[3].value = 6.0;
            impulse_through(g, f, 64);
        });
        check(
            &out[0],
            &[0, 1, 2, 3, 10],
            &want,
            2e-6,
            &format!("{kind:?}"),
        );
    }
}

#[test]
fn delay_panners_mixing_rate_loop_shaper_and_convolver_are_the_reference_browsers() {
    // A 10 ms delay moves an impulse by 480 frames.
    let out = render(2048, 1, |g| {
        let d = g.add(nodes::delay(1.0, R)).unwrap();
        g.node_mut(d).unwrap().params[0].value = 0.01;
        let s = g.add(nodes::buffer_source()).unwrap();
        if let Kind::BufferSource(b) = &mut g.node_mut(s).unwrap().kind {
            b.buffer = Some(buffer(&[1.0, 0.5, 0.25, 0.125]));
        }
        started(g, s);
        g.connect(s, 0, d, 0);
        g.connect(d, 0, 0, 0);
    });
    check(
        &out[0],
        &[0, 479, 480, 481, 482, 483, 484],
        &[0.0, 0.0, 1.0, 0.5, 0.25, 0.125, 0.0],
        1e-6,
        "delay",
    );
    // A stereo panner at 0.5 on a mono 1.
    let out = render(128, 2, |g| {
        let s = g.add(nodes::constant()).unwrap();
        let p = g.add(nodes::stereo_panner()).unwrap();
        g.node_mut(p).unwrap().params[0].value = 0.5;
        started(g, s);
        g.connect(s, 0, p, 0);
        g.connect(p, 0, 0, 0);
    });
    check(&out[0], &[0], &[0.382683], 1e-6, "pan left");
    check(&out[1], &[0], &[0.923880], 1e-6, "pan right");
    // Mono to stereo: copies.
    let out = render(128, 2, |g| {
        let s = g.add(nodes::constant()).unwrap();
        g.node_mut(s).unwrap().params[0].value = 0.5;
        started(g, s);
        g.connect(s, 0, 0, 0);
    });
    assert_eq!((out[0][0], out[1][0]), (0.5, 0.5));
    // playbackRate 0.5 over 0, 1, ... 15: linear, the last step extended.
    let values: Vec<f32> = (0..16).map(|v| v as f32).collect();
    let out = render(128, 1, |g| {
        let s = g.add(nodes::buffer_source()).unwrap();
        let node = g.node_mut(s).unwrap();
        node.params[0].value = 0.5;
        if let Kind::BufferSource(b) = &mut node.kind {
            b.buffer = Some(buffer(&values));
        }
        started(g, s);
        g.connect(s, 0, 0, 0);
    });
    check(
        &out[0],
        &[0, 1, 2, 3, 4, 5, 29, 30, 31, 32],
        &[0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 14.5, 15.0, 15.5, 0.0],
        1e-6,
        "rate",
    );
    // A loop between frames 1 and 3.
    let out = render(128, 1, |g| {
        let s = g.add(nodes::buffer_source()).unwrap();
        if let Kind::BufferSource(b) = &mut g.node_mut(s).unwrap().kind {
            b.buffer = Some(buffer(&[1.0, 2.0, 3.0, 4.0]));
            b.looping = true;
            b.loop_start = 1.0 / f64::from(R);
            b.loop_end = 3.0 / f64::from(R);
        }
        started(g, s);
        g.connect(s, 0, 0, 0);
    });
    check(
        &out[0],
        &[0, 1, 2, 3, 4, 5, 6, 7],
        &[1.0, 2.0, 3.0, 2.0, 3.0, 2.0, 3.0, 2.0],
        1e-6,
        "loop",
    );
    // A shaper's curve, interpolated.
    let out = render(128, 1, |g| {
        let w = g.add(nodes::shaper()).unwrap();
        if let Kind::Shaper(s) = &mut g.node_mut(w).unwrap().kind {
            s.curve = Some(vec![0.0, 0.5, 1.0]);
        }
        let s = g.add(nodes::buffer_source()).unwrap();
        if let Kind::BufferSource(b) = &mut g.node_mut(s).unwrap().kind {
            b.buffer = Some(buffer(&[-1.0, -0.5, 0.25, 1.0]));
        }
        started(g, s);
        g.connect(s, 0, w, 0);
        g.connect(w, 0, 0, 0);
    });
    check(
        &out[0],
        &[0, 1, 2, 3],
        &[0.0, 0.25, 0.625, 1.0],
        1e-6,
        "shaper",
    );
    // A convolver, plain and normalized.
    for (normalize, want) in [
        (false, [0.5, 0.25, 0.125, 0.0, 0.0]),
        (true, [0.001749, 0.000874, 0.000437, 0.0, 0.0]),
    ] {
        let out = render(128, 1, |g| {
            let c = g.add(nodes::convolver()).unwrap();
            if let Kind::Convolver(conv) = &mut g.node_mut(c).unwrap().kind {
                conv.set_response(
                    Some(&AudioData {
                        rate: R,
                        channels: vec![vec![0.5, 0.25, 0.125]],
                    }),
                    normalize,
                );
            }
            impulse_through(g, c, 4);
        });
        check(&out[0], &[0, 1, 2, 3, 4], &want, 1e-6, "convolver");
    }
    // An equal-power panner at x = 1: all right.
    let out = render(128, 2, |g| {
        let s = g.add(nodes::constant()).unwrap();
        let p = g.add(nodes::panner()).unwrap();
        g.node_mut(p).unwrap().params[0].value = 1.0;
        started(g, s);
        g.connect(s, 0, p, 0);
        g.connect(p, 0, 0, 0);
    });
    assert!(
        out[0][0].abs() < 1e-6 && (out[1][0] - 1.0).abs() < 1e-6,
        "{} {}",
        out[0][0],
        out[1][0]
    );
}

#[test]
fn a_long_response_convolves_by_partitions_as_it_would_directly() {
    let response: Vec<f32> = (0..1000)
        .map(|i| ((i * 7919) % 101) as f32 / 101.0 - 0.5)
        .collect();
    let input: Vec<f32> = (0..2048).map(|i| ((i as f32) * 0.37).sin()).collect();
    let out = render(2048, 1, |g| {
        let c = g.add(nodes::convolver()).unwrap();
        if let Kind::Convolver(conv) = &mut g.node_mut(c).unwrap().kind {
            conv.set_response(
                Some(&AudioData {
                    rate: R,
                    channels: vec![response.clone()],
                }),
                false,
            );
        }
        let s = g.add(nodes::buffer_source()).unwrap();
        if let Kind::BufferSource(b) = &mut g.node_mut(s).unwrap().kind {
            b.buffer = Some(buffer(&input));
        }
        started(g, s);
        g.connect(s, 0, c, 0);
        g.connect(c, 0, 0, 0);
    });
    for n in [0, 1, 127, 128, 999, 1000, 1500, 2047] {
        let want: f32 = (0..=n)
            .filter(|&k| n - k < response.len())
            .map(|k| input[k] * response[n - k])
            .sum();
        assert!(
            (out[0][n] - want).abs() < 1e-3,
            "{n}: {} for {want}",
            out[0][n]
        );
    }
}

#[test]
fn a_cycle_sounds_through_a_delay_and_is_silent_without_one() {
    // A gain feeding itself, a source into it: muted.
    let out = render(256, 1, |g| {
        let s = g.add(nodes::constant()).unwrap();
        let a = g.add(nodes::gain()).unwrap();
        let b = g.add(nodes::gain()).unwrap();
        started(g, s);
        g.connect(s, 0, a, 0);
        g.connect(a, 0, b, 0);
        g.connect(b, 0, a, 0);
        g.connect(a, 0, 0, 0);
    });
    assert!(out[0].iter().all(|v| *v == 0.0));
    // A muted cycle silences itself, not what it feeds.
    let out = render(256, 1, |g| {
        let s = g.add(nodes::constant()).unwrap();
        g.node_mut(s).unwrap().params[0].value = 0.25;
        let a = g.add(nodes::gain()).unwrap();
        let b = g.add(nodes::gain()).unwrap();
        started(g, s);
        g.connect(a, 0, b, 0);
        g.connect(b, 0, a, 0);
        g.connect(a, 0, 0, 0);
        g.connect(s, 0, 0, 0);
    });
    assert!(out[0].iter().all(|v| *v == 0.25));
    // A delay of 1 ms in a cycle delays a quantum.
    let out = render(512, 1, |g| {
        let s = g.add(nodes::constant()).unwrap();
        let mix = g.add(nodes::gain()).unwrap();
        let d = g.add(nodes::delay(1.0, R)).unwrap();
        g.node_mut(d).unwrap().params[0].value = 0.001;
        started(g, s);
        g.connect(s, 0, mix, 0);
        g.connect(mix, 0, d, 0);
        g.connect(d, 0, mix, 0);
        g.connect(mix, 0, 0, 0);
    });
    assert_eq!((out[0][127], out[0][128], out[0][256]), (1.0, 2.0, 3.0));
    // Through a delay of a quantum at a gain of a half: 1, then 1.5,
    // 1.75 a quantum later each.
    let out = render(512, 1, |g| {
        let s = g.add(nodes::constant()).unwrap();
        let mix = g.add(nodes::gain()).unwrap();
        let d = g.add(nodes::delay(1.0, R)).unwrap();
        g.node_mut(d).unwrap().params[0].value = 128.0 / R;
        let half = g.add(nodes::gain()).unwrap();
        g.node_mut(half).unwrap().params[0].value = 0.5;
        started(g, s);
        g.connect(s, 0, mix, 0);
        g.connect(mix, 0, d, 0);
        g.connect(d, 0, half, 0);
        g.connect(half, 0, mix, 0);
        g.connect(mix, 0, 0, 0);
    });
    assert_eq!(
        (out[0][0], out[0][128], out[0][256], out[0][384]),
        (1.0, 1.5, 1.75, 1.875)
    );
}

#[test]
fn connections_are_checked_and_disconnections_counted() {
    let mut g = Graph::new(R, 2);
    let a = g.add(nodes::gain()).unwrap();
    let b = g.add(nodes::gain()).unwrap();
    assert!(!g.connect(a, 1, b, 0));
    assert!(!g.connect(a, 0, b, 1));
    assert!(!g.connect(a, 0, 99, 0));
    assert!(g.connect(a, 0, b, 0) && g.connect(a, 0, b, 0));
    assert!(g.connect_param(a, 0, b, 0));
    assert_eq!(g.disconnect(a, None, Some(b), None, false), 1);
    assert_eq!(g.disconnect(a, None, Some(b), None, false), 0);
    assert_eq!(g.disconnect(a, None, None, None, true), 1);
    g.remove(b);
    assert!(!g.connect(a, 0, b, 0));
}

#[test]
fn sources_end_once_and_say_so() {
    let mut g = Graph::new(R, 1);
    let s = g.add(nodes::buffer_source()).unwrap();
    if let Kind::BufferSource(b) = &mut g.node_mut(s).unwrap().kind {
        b.buffer = Some(buffer(&[1.0; 200]));
    }
    started(&mut g, s);
    g.connect(s, 0, 0, 0);
    g.render();
    assert!(g.take_ended().is_empty());
    let out = g.render();
    assert_eq!((out[0][71], out[0][72]), (1.0, 0.0));
    assert_eq!(g.take_ended(), vec![s]);
    g.render();
    assert!(g.take_ended().is_empty());
}

#[test]
fn the_analyser_finds_a_tones_bin() {
    let mut g = Graph::new(R, 1);
    let o = g.add(nodes::oscillator(R)).unwrap();
    g.node_mut(o).unwrap().params[0].value = 1500.0;
    started(&mut g, o);
    let a = g.add(nodes::analyser()).unwrap();
    g.connect(o, 0, a, 0);
    g.connect(a, 0, 0, 0);
    for _ in 0..40 {
        g.render();
    }
    let Kind::Analyser(analyser) = &g.node(a).unwrap().kind else {
        unreachable!()
    };
    let mut data = analyser.shared.lock().unwrap();
    let mut db = vec![0.0f32; 1024];
    data.float_frequency(&mut db);
    let peak = db
        .iter()
        .enumerate()
        .fold(
            (0, f32::MIN),
            |m, (i, v)| if *v > m.1 { (i, *v) } else { m },
        )
        .0;
    // 1500 Hz in bins of 48000 / 2048 Hz.
    assert_eq!(peak, 64);
    let mut time = vec![0.0f32; 2048];
    data.float_time(&mut time);
    assert!(time.iter().any(|v| *v > 0.9));
}

#[test]
fn the_compressor_looks_ahead_then_reduces_a_loud_input() {
    let out = render(4800, 1, |g| {
        let s = g.add(nodes::constant()).unwrap();
        g.node_mut(s).unwrap().params[0].value = 0.9;
        let c = g.add(nodes::compressor(R)).unwrap();
        started(g, s);
        g.connect(s, 0, c, 0);
        g.connect(c, 0, 0, 0);
    });
    // Silent through its 6 ms look-ahead, as the reference's.
    assert!(out[0][..288].iter().all(|v| *v == 0.0));
    let last = out[0][4799];
    assert!(last > 0.0 && last < 0.9 * 2.0, "{last}");
}

#[test]
fn a_splitter_and_a_merger_swap_channels() {
    let out = render(128, 2, |g| {
        let l = g.add(nodes::constant()).unwrap();
        g.node_mut(l).unwrap().params[0].value = 0.25;
        let r = g.add(nodes::constant()).unwrap();
        g.node_mut(r).unwrap().params[0].value = 0.75;
        let m1 = g.add(nodes::merger(2)).unwrap();
        let sp = g.add(nodes::splitter(2)).unwrap();
        let m2 = g.add(nodes::merger(2)).unwrap();
        started(g, l);
        started(g, r);
        g.connect(l, 0, m1, 0);
        g.connect(r, 0, m1, 1);
        g.connect(m1, 0, sp, 0);
        g.connect(sp, 0, m2, 1);
        g.connect(sp, 1, m2, 0);
        g.connect(m2, 0, 0, 0);
    });
    assert_eq!((out[0][5], out[1][5]), (0.75, 0.25));
}
