//! `AudioParam` (Web Audio 1.1 section 1.6): its automation timeline and
//! the value it computes at each frame, its inputs added and the sum
//! clamped to its nominal range.

/// Whether a param takes a value for every frame or one a quantum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rate {
    A,
    K,
}

/// An automation event, its times in seconds.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    SetValue {
        value: f32,
        time: f64,
    },
    LinearRamp {
        value: f32,
        time: f64,
    },
    ExponentialRamp {
        value: f32,
        time: f64,
    },
    SetTarget {
        target: f32,
        time: f64,
        constant: f64,
    },
    SetValueCurve {
        values: std::sync::Arc<[f32]>,
        time: f64,
        duration: f64,
    },
}

impl Event {
    pub fn time(&self) -> f64 {
        match self {
            Event::SetValue { time, .. }
            | Event::LinearRamp { time, .. }
            | Event::ExponentialRamp { time, .. }
            | Event::SetTarget { time, .. }
            | Event::SetValueCurve { time, .. } => *time,
        }
    }

    /// Where its effect on the timeline ends: a curve's end, else its
    /// time.
    fn end(&self) -> f64 {
        match self {
            Event::SetValueCurve { time, duration, .. } => time + duration,
            other => other.time(),
        }
    }
}

/// Why an event could not be scheduled: it overlaps a value curve
/// (`NotSupportedError` in the bindings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlap;

/// A param's timeline and value.
#[derive(Debug, Clone)]
pub struct Param {
    /// Its value with no events: the `value` attribute as last set.
    pub value: f32,
    pub default: f32,
    pub min: f32,
    pub max: f32,
    pub rate: Rate,
    /// Events by time; at one time, in the order scheduled.
    events: Vec<Event>,
    /// When the first event's ramp, if it is one, starts: the time it was
    /// scheduled, or the end of the events dropped before it. Its value
    /// there is `value`.
    origin: f64,
}

impl Param {
    pub fn new(default: f32, min: f32, max: f32, rate: Rate) -> Param {
        Param {
            value: default,
            default,
            min,
            max,
            rate,
            events: Vec::new(),
            origin: 0.0,
        }
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Adds `event`, scheduled at `now` (the context's current time).
    pub fn schedule(&mut self, event: Event, now: f64) -> Result<(), Overlap> {
        // A curve may not overlap another event, nor an event a curve.
        let (start, end) = (event.time(), event.end());
        for other in &self.events {
            let overlaps = match (&event, other) {
                (Event::SetValueCurve { .. }, _) => {
                    let t = other.time();
                    (t > start && t < end)
                        || (matches!(other, Event::SetValueCurve { .. })
                            && other.time() <= start
                            && other.end() > start)
                }
                (_, Event::SetValueCurve { .. }) => start > other.time() && start < other.end(),
                _ => false,
            };
            if overlaps {
                return Err(Overlap);
            }
        }
        if matches!(event, Event::SetValueCurve { .. })
            && self.events.iter().any(|e| {
                (e.time() - start).abs() < f64::EPSILON && !matches!(e, Event::SetValueCurve { .. })
            })
        {
            return Err(Overlap);
        }
        // After the last event at the same time, before any later.
        let at = self.events.partition_point(|e| e.time() <= start);
        if at == 0 {
            self.origin = now;
        }
        self.events.insert(at, event);
        Ok(())
    }

    /// `cancelScheduledValues(time)`: every event at or after `time` gone.
    pub fn cancel(&mut self, time: f64) {
        self.events.retain(|e| e.time() < time);
    }

    /// `cancelAndHoldAtTime(time)`: the events after `time` gone and the
    /// value at `time` held from it, as section 1.6.2's steps say.
    pub fn cancel_and_hold(&mut self, time: f64, sample_rate: f32) {
        let held = self.value_at(time);
        // A ramp in progress at `time` is cut there, ending at its value.
        let next = self.events.iter().position(|e| e.time() > time);
        let mut replacement = None;
        if let Some(i) = next {
            match &self.events[i] {
                Event::LinearRamp { .. } => {
                    replacement = Some(Event::LinearRamp { value: held, time })
                }
                Event::ExponentialRamp { .. } => {
                    replacement = Some(Event::ExponentialRamp { value: held, time })
                }
                _ => {}
            }
        }
        // A curve in progress is cut short.
        let mut curve_cut = None;
        if let Some(i) = self.events.iter().rposition(|e| e.time() <= time)
            && let Event::SetValueCurve {
                values,
                time: start,
                duration,
            } = &self.events[i]
            && start + duration > time
        {
            curve_cut = Some((i, values.clone(), *start, time - start));
        }
        self.events.retain(|e| e.time() <= time);
        let _ = sample_rate;
        if let Some((i, values, start, duration)) = curve_cut {
            self.events[i] = Event::SetValueCurve {
                values,
                time: start,
                duration,
            };
            self.events.push(Event::SetValue {
                value: held,
                time: start + duration,
            });
        } else if let Some(event) = replacement {
            self.events.push(event);
        } else if let Some(Event::SetTarget { .. }) = self.events.last() {
            self.events.push(Event::SetValue { value: held, time });
        } else {
            self.events.push(Event::SetValue { value: held, time });
        }
    }

    /// The events at or before `time` that can no longer change a value
    /// to come dropped, keeping the timeline short; `value` is then the
    /// value the last of them left.
    pub fn forget_before(&mut self, time: f64) {
        // The last event wholly past at `time` stays, with what follows;
        // the value and end of the one before it become the origin.
        let Some(last) = self.events.iter().rposition(|e| e.end() <= time) else {
            return;
        };
        if last == 0 {
            return;
        }
        let value = self.value_at_event_end(last - 1);
        self.origin = self.events[last - 1].end();
        self.value = value;
        self.events.drain(..last);
    }

    /// The value the timeline gives at `time`, its inputs aside.
    pub fn value_at(&self, time: f64) -> f32 {
        let events = &self.events;
        if events.is_empty() {
            return self.value;
        }
        // The last event at or before `time`, and the one after.
        let next = events.partition_point(|e| e.time() <= time);
        // The value and time before the event at index `i`: what it
        // ramps from.
        let before = |i: usize| -> (f32, f64) {
            if i == 0 {
                return (self.value, self.origin);
            }
            let prev = &events[i - 1];
            let end = prev.end();
            (self.value_at_event_end(i - 1), end)
        };
        if next < events.len() {
            match &events[next] {
                Event::LinearRamp { value, time: t1 } => {
                    let (v0, t0) = before(next);
                    if time < t0 || *t1 <= t0 {
                        return v0;
                    }
                    return v0 + (value - v0) * ((time - t0) / (t1 - t0)) as f32;
                }
                Event::ExponentialRamp { value, time: t1 } => {
                    let (v0, t0) = before(next);
                    if time < t0 || *t1 <= t0 {
                        return v0;
                    }
                    if v0 == 0.0 || (v0 > 0.0) != (*value > 0.0) {
                        return v0;
                    }
                    let ratio = f64::from(*value / v0);
                    return (f64::from(v0) * ratio.powf((time - t0) / (t1 - t0))) as f32;
                }
                _ => {}
            }
        }
        if next == 0 {
            return self.value;
        }
        let i = next - 1;
        match &events[i] {
            Event::SetValue { value, .. }
            | Event::LinearRamp { value, .. }
            | Event::ExponentialRamp { value, .. } => *value,
            Event::SetTarget {
                target,
                time: t0,
                constant,
            } => {
                let v0 = before(i).0;
                if *constant <= 0.0 {
                    return *target;
                }
                (f64::from(*target) + f64::from(v0 - target) * (-(time - t0) / constant).exp())
                    as f32
            }
            Event::SetValueCurve {
                values,
                time: t0,
                duration,
            } => curve(values, *t0, *duration, time),
        }
    }

    /// The value the event at `i` leaves at its end, for what follows it.
    fn value_at_event_end(&self, i: usize) -> f32 {
        match &self.events[i] {
            Event::SetValue { value, .. }
            | Event::LinearRamp { value, .. }
            | Event::ExponentialRamp { value, .. } => *value,
            Event::SetValueCurve { values, .. } => values.last().copied().unwrap_or(self.value),
            Event::SetTarget {
                target,
                time: t0,
                constant,
            } => {
                // Its value at the time the next event starts.
                let at = self.events.get(i + 1).map_or(*t0, Event::time);
                let v0 = if i == 0 {
                    self.value
                } else {
                    self.value_at_event_end(i - 1)
                };
                if *constant <= 0.0 {
                    return *target;
                }
                (f64::from(*target) + f64::from(v0 - target) * (-(at - t0) / constant).exp()) as f32
            }
        }
    }

    /// The intrinsic values for `out.len()` frames from frame `start` at
    /// `sample_rate`, each its frame's time: every frame for an a-rate
    /// param, the first frame's for every frame of a k-rate one.
    pub fn intrinsic(&self, start: u64, sample_rate: f32, out: &mut [f32]) {
        let rate = f64::from(sample_rate);
        if self.events.is_empty() || self.rate == Rate::K {
            let v = self.value_at(start as f64 / rate);
            out.fill(v);
            return;
        }
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = self.value_at((start + i as u64) as f64 / rate);
        }
    }

    /// The computed values: the intrinsic values plus `inputs` (the mono
    /// mix of what is connected to it, if any), clamped to the nominal
    /// range.
    pub fn computed(&self, start: u64, sample_rate: f32, inputs: Option<&[f32]>, out: &mut [f32]) {
        self.intrinsic(start, sample_rate, out);
        if let Some(inputs) = inputs {
            if self.rate == Rate::K {
                let add = inputs.first().copied().unwrap_or(0.0);
                for v in out.iter_mut() {
                    *v += add;
                }
            } else {
                for (v, x) in out.iter_mut().zip(inputs) {
                    *v += x;
                }
            }
        }
        for v in out.iter_mut() {
            *v = if v.is_nan() {
                self.default.clamp(self.min, self.max)
            } else {
                v.clamp(self.min, self.max)
            };
        }
    }
}

/// A value curve's value at `time`: interpolated linearly between its
/// points, its last value after its end.
fn curve(values: &[f32], start: f64, duration: f64, time: f64) -> f32 {
    let n = values.len();
    if n == 0 {
        return 0.0;
    }
    if time >= start + duration || n == 1 || duration <= 0.0 {
        return values[n - 1];
    }
    let step = duration / (n - 1) as f64;
    let at = ((time - start) / step).max(0.0);
    let k = (at.floor() as usize).min(n - 2);
    let frac = (at - k as f64) as f32;
    values[k] + (values[k + 1] - values[k]) * frac
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Probe p5: setValueAtTime(0, 0), linearRampToValueAtTime(1, 0.02),
    /// exponentialRampToValueAtTime(0.01, 0.06), setTargetAtTime(0.5,
    /// 0.07, 0.005), at 48 kHz, and the reference's values.
    #[test]
    fn the_automation_curves_are_the_reference_browsers() {
        let mut p = Param::new(1.0, f32::MIN, f32::MAX, Rate::A);
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
        let want = [
            (0, 0.0),
            (1, 0.001042),
            (480, 0.5),
            (959, 0.998958),
            (960, 1.0),
            (1440, 0.316228),
            (2000, 0.082540),
            (2879, 0.010024),
            (3360, 0.01),
            (3600, 0.319739),
            (4000, 0.465953),
            (4799, 0.498780),
        ];
        let mut out = vec![0.0; 4800];
        p.computed(0, 48_000.0, None, &mut out);
        for (frame, v) in want {
            assert!(
                (out[frame] - v).abs() < 2e-6,
                "{frame}: {} for {v}",
                out[frame]
            );
        }
    }

    #[test]
    fn a_curve_interpolates_and_holds_its_last_value_and_may_not_be_overlapped() {
        let mut p = Param::new(0.0, -10.0, 10.0, Rate::A);
        p.schedule(
            Event::SetValueCurve {
                values: vec![0.0, 1.0, 0.5].into(),
                time: 1.0,
                duration: 2.0,
            },
            0.0,
        )
        .unwrap();
        assert_eq!(p.value_at(0.5), 0.0);
        assert_eq!(p.value_at(1.5), 0.5);
        assert_eq!(p.value_at(2.5), 0.75);
        assert_eq!(p.value_at(9.0), 0.5);
        assert_eq!(
            p.schedule(
                Event::SetValue {
                    value: 1.0,
                    time: 2.0
                },
                0.0
            ),
            Err(Overlap)
        );
        assert!(
            p.schedule(
                Event::SetValue {
                    value: 1.0,
                    time: 3.0
                },
                0.0
            )
            .is_ok()
        );
    }

    #[test]
    fn inputs_add_and_the_sum_is_clamped_k_rate_once_a_quantum() {
        let mut p = Param::new(0.5, 0.0, 1.0, Rate::K);
        p.schedule(
            Event::LinearRamp {
                value: 1.0,
                time: 1.0,
            },
            0.0,
        )
        .unwrap();
        let mut out = [0.0; 4];
        p.computed(24_000, 48_000.0, Some(&[0.2, 0.9, 0.9, 0.9]), &mut out);
        // 0.75 at 0.5 s, plus the first input, the same for each frame.
        assert!(out.iter().all(|v| (v - 0.95).abs() < 1e-6), "{out:?}");
        p.computed(24_000, 48_000.0, Some(&[0.9; 4]), &mut out);
        assert_eq!(out, [1.0; 4]);
    }

    #[test]
    fn cancel_and_hold_cuts_a_ramp_at_its_value() {
        let mut p = Param::new(0.0, -10.0, 10.0, Rate::A);
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
                time: 1.0,
            },
            0.0,
        )
        .unwrap();
        p.cancel_and_hold(0.5, 48_000.0);
        assert!((p.value_at(0.25) - 0.25).abs() < 1e-6);
        assert!((p.value_at(0.75) - 0.5).abs() < 1e-6);
        p.cancel(0.0);
        assert!(p.events().is_empty());
    }

    #[test]
    fn forgetting_past_events_keeps_every_value_to_come() {
        let mut p = Param::new(0.0, -10.0, 10.0, Rate::A);
        let mut q = p.clone();
        for (i, param) in [&mut p, &mut q].into_iter().enumerate() {
            let _ = i;
            for k in 0..20 {
                let t = f64::from(k) * 0.1;
                param
                    .schedule(
                        Event::SetValue {
                            value: k as f32,
                            time: t,
                        },
                        0.0,
                    )
                    .unwrap();
                param
                    .schedule(
                        Event::LinearRamp {
                            value: k as f32 + 0.5,
                            time: t + 0.05,
                        },
                        0.0,
                    )
                    .unwrap();
            }
            param
                .schedule(
                    Event::SetTarget {
                        target: 3.0,
                        time: 2.0,
                        constant: 0.1,
                    },
                    0.0,
                )
                .unwrap();
        }
        q.forget_before(1.23);
        assert!(q.events().len() < p.events().len());
        for k in 124..260 {
            let t = f64::from(k) * 0.01;
            assert!((p.value_at(t) - q.value_at(t)).abs() < 1e-5, "{t}");
        }
    }
}
