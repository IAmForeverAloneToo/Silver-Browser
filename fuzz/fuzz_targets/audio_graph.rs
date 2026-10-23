//! Web Audio's rendering (M40) driven by arbitrary bytes: any sequence of
//! node creations, connections to nodes and to params, disconnections,
//! removals, param values and automation events, node settings and
//! rendered quanta. Nothing may panic, the graph keeps `MAX_NODES`, and
//! each quantum the destination gives its channels, a quantum each.

#![no_main]
#![forbid(unsafe_code)]

use std::sync::Arc;

use audio::nodes::{self, Kind};
use audio::{AudioData, Event, Graph, MAX_NODES, QUANTUM};
use libfuzzer_sys::fuzz_target;

const R: f32 = 48_000.0;

/// Reads the input a byte or a few at a time.
struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn byte(&mut self) -> Option<u8> {
        let (&b, rest) = self.0.split_first()?;
        self.0 = rest;
        Some(b)
    }

    fn small(&mut self, below: usize) -> Option<usize> {
        Some(usize::from(self.byte()?) % below.max(1))
    }

    /// A value from -16 to 16, in steps of an eighth.
    fn value(&mut self) -> Option<f32> {
        Some((f32::from(self.byte()?) - 128.0) / 8.0)
    }
}

fuzz_target!(|data: &[u8]| {
    let mut input = Bytes(data);
    let channels = input.small(6).unwrap_or(1) + 1;
    let mut graph = Graph::new(R, channels);
    let mut ids: Vec<u32> = vec![0];
    let mut rendered = 0;
    while let Some(op) = input.byte() {
        match op % 10 {
            0 | 1 => {
                let Some(kind) = input.small(15) else { break };
                let node = match kind {
                    0 => nodes::gain(),
                    1 => nodes::constant(),
                    2 => nodes::oscillator(R),
                    3 => nodes::buffer_source(),
                    4 => nodes::biquad(R),
                    5 => nodes::iir(vec![0.5, 0.25], vec![1.0, -0.5]),
                    6 => nodes::delay(1.0, R),
                    7 => nodes::stereo_panner(),
                    8 => nodes::panner(),
                    9 => nodes::splitter(input.small(6).unwrap_or(0) + 1),
                    10 => nodes::merger(input.small(6).unwrap_or(0) + 1),
                    11 => nodes::shaper(),
                    12 => nodes::compressor(R),
                    13 => nodes::convolver(),
                    _ => nodes::analyser(),
                };
                if let Some(id) = graph.add(node) {
                    ids.push(id);
                }
            }
            2 | 3 => {
                let (Some(a), Some(o), Some(b), Some(i)) = (
                    input.small(ids.len()),
                    input.small(6),
                    input.small(ids.len()),
                    input.small(6),
                ) else {
                    break;
                };
                if op % 10 == 2 {
                    graph.connect(ids[a], o, ids[b], i);
                } else {
                    graph.connect_param(ids[a], o, ids[b], i);
                }
            }
            4 => {
                let (Some(a), Some(b)) = (input.small(ids.len()), input.small(ids.len())) else {
                    break;
                };
                graph.disconnect(ids[a], None, Some(ids[b]), None, b % 2 == 0);
            }
            5 => {
                let Some(a) = input.small(ids.len()) else {
                    break;
                };
                graph.remove(ids[a]);
            }
            6 => {
                let (Some(a), Some(p), Some(kind), Some(v), Some(t)) = (
                    input.small(ids.len()),
                    input.small(6),
                    input.small(5),
                    input.value(),
                    input.byte(),
                ) else {
                    break;
                };
                let now = graph.current_time();
                let time = now + f64::from(t) / 256.0;
                if let Some(node) = graph.node_mut(ids[a])
                    && let Some(param) = node.params.get_mut(p)
                {
                    let event = match kind {
                        0 => Event::SetValue { value: v, time },
                        1 => Event::LinearRamp { value: v, time },
                        2 => Event::ExponentialRamp {
                            value: if v == 0.0 { 0.5 } else { v },
                            time,
                        },
                        3 => Event::SetTarget {
                            target: v,
                            time,
                            constant: f64::from(t) / 1024.0,
                        },
                        _ => Event::SetValueCurve {
                            values: vec![v, -v, v * 0.5].into(),
                            time,
                            duration: 0.01,
                        },
                    };
                    let _ = param.schedule(event, now);
                    if kind == 4 {
                        param.cancel_and_hold(time + 0.005, R);
                    }
                }
            }
            7 => {
                let (Some(a), Some(v)) = (input.small(ids.len()), input.value()) else {
                    break;
                };
                let Some(node) = graph.node_mut(ids[a]) else {
                    continue;
                };
                match &mut node.kind {
                    Kind::Constant(s) => s.start = Some(0.0),
                    Kind::Oscillator(o) => o.schedule.start = Some(f64::from(v.abs()) / 100.0),
                    Kind::BufferSource(b) => {
                        b.buffer = Some(Arc::new(AudioData {
                            rate: 22_050.0 + v.abs() * 1000.0,
                            channels: vec![vec![v / 16.0; 300]; (v.abs() as usize % 3) + 1],
                        }));
                        b.looping = v > 0.0;
                        b.loop_start = 0.001;
                        b.loop_end = f64::from(v.abs()) / 1000.0;
                        b.schedule.start = Some(0.0);
                    }
                    Kind::Shaper(s) => s.curve = Some(vec![v, -v, 0.5, v * 0.25]),
                    Kind::Convolver(c) => c.set_response(
                        Some(&AudioData {
                            rate: R,
                            channels: vec![vec![v / 16.0; 400]; [1, 2, 4][v.abs() as usize % 3]],
                        }),
                        v > 0.0,
                    ),
                    _ => {}
                }
                if let Some(param) = node.params.first_mut() {
                    param.value = v;
                }
            }
            _ => {
                let Some(n) = input.small(4) else { break };
                for _ in 0..=n {
                    if rendered >= 64 {
                        break;
                    }
                    let bus = graph.render();
                    rendered += 1;
                    assert_eq!(bus.len(), channels);
                    assert!(bus.iter().all(|c| c.len() == QUANTUM));
                    let _ = graph.take_ended();
                }
            }
        }
        assert!(ids.len() <= MAX_NODES + 1);
    }
});
