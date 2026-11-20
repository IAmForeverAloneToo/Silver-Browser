//! The network's quality as `navigator.connection` tells it (2026-11-20):
//! the round trip and the throughput this renderer's fetches measured
//! over the public network, as the reference's network quality estimate
//! keeps them, local and loopback servers left out; and what a page is
//! told of them, rounded, capped and moved by a factor of its host's, as
//! the reference tells them (`network_state_notifier.cc`). Until a
//! public response is measured, the reference's estimate for a profile
//! that measured none (probed, Chrome 157: 100 ms, 1.6 Mbps).

use std::collections::VecDeque;
use std::hash::BuildHasher;
use std::sync::OnceLock;
use std::time::Duration;

/// How many samples of each kind the estimate is the median of.
const SAMPLES: usize = 20;
/// The estimate before any sample.
const DEFAULT_RTT_MS: f64 = 100.0;
const DEFAULT_DOWNLINK_MBPS: f64 = 1.6;
/// A response this small times a round trip, its body nothing to speak
/// of; one at least this big times the throughput.
const RTT_BODY_MOST: usize = 4 * 1024;
const THROUGHPUT_BODY_LEAST: usize = 32 * 1024;
/// What a page is told at most, and to what steps.
const MAX_RTT_MS: f64 = 3000.0;
const RTT_STEP_MS: f64 = 50.0;
const MAX_DOWNLINK_MBPS: f64 = 10.0;
const DOWNLINK_STEP_KBPS: f64 = 50.0;

/// The samples taken.
#[derive(Debug, Default)]
pub(crate) struct Quality {
    rtts: VecDeque<f64>,
    downlinks: VecDeque<f64>,
}

impl Quality {
    /// A response of the public network fetched, its head `head` after
    /// the request and its whole `whole` after, its body `bytes` long.
    pub(crate) fn measured(&mut self, head: Duration, whole: Duration, bytes: usize) {
        if bytes <= RTT_BODY_MOST {
            push(&mut self.rtts, head.as_secs_f64() * 1000.0);
        }
        let transfer = whole.saturating_sub(head).as_secs_f64();
        if bytes >= THROUGHPUT_BODY_LEAST && transfer > 0.0 {
            push(
                &mut self.downlinks,
                bytes as f64 * 8.0 / transfer / 1_000_000.0,
            );
        }
    }

    /// The estimate: the round trip in milliseconds and the throughput
    /// in megabits a second.
    pub(crate) fn estimate(&self) -> (f64, f64) {
        (
            median(&self.rtts).unwrap_or(DEFAULT_RTT_MS),
            median(&self.downlinks).unwrap_or(DEFAULT_DOWNLINK_MBPS),
        )
    }
}

fn push(samples: &mut VecDeque<f64>, value: f64) {
    if !value.is_finite() || value < 0.0 {
        return;
    }
    if samples.len() == SAMPLES {
        samples.pop_front();
    }
    samples.push_back(value);
}

fn median(samples: &VecDeque<f64>) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted: Vec<f64> = samples.iter().copied().collect();
    sorted.sort_by(f64::total_cmp);
    Some(sorted[sorted.len() / 2])
}

/// The effective type of an estimate, by the Network Information API's
/// table: `slow-2g`, `2g`, `3g` or `4g`.
pub(crate) fn effective_type(rtt_ms: f64, downlink_mbps: f64) -> &'static str {
    if rtt_ms >= 2000.0 || downlink_mbps < 0.05 {
        "slow-2g"
    } else if rtt_ms >= 1400.0 || downlink_mbps < 0.07 {
        "2g"
    } else if rtt_ms >= 270.0 || downlink_mbps < 0.7 {
        "3g"
    } else {
        "4g"
    }
}

/// The factor a host's pages see an estimate moved by, in [0.9, 1.1]:
/// one a host, from a key this process draws once, as the reference's
/// is one a host and its browser's.
fn factor(host: &str) -> f64 {
    static KEY: OnceLock<std::collections::hash_map::RandomState> = OnceLock::new();
    let hash = KEY.get_or_init(Default::default).hash_one(host);
    0.9 + (hash % 2001) as f64 / 10_000.0
}

/// What a page of `host` is told of an estimate: its effective type, its
/// round trip in milliseconds and its throughput in megabits a second,
/// moved by the host's factor, capped and rounded to the reference's
/// steps.
pub(crate) fn told(host: &str, (rtt_ms, downlink_mbps): (f64, f64)) -> (&'static str, f64, f64) {
    let factor = factor(host);
    let rtt = ((rtt_ms * factor).min(MAX_RTT_MS) / RTT_STEP_MS).round() * RTT_STEP_MS;
    let kbps = (downlink_mbps * factor).min(MAX_DOWNLINK_MBPS) * 1000.0;
    let downlink = (kbps / DOWNLINK_STEP_KBPS).round() * DOWNLINK_STEP_KBPS / 1000.0;
    (effective_type(rtt_ms, downlink_mbps), rtt, downlink)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_estimate_is_the_median_of_the_samples_or_the_references_default() {
        let mut quality = Quality::default();
        assert_eq!(quality.estimate(), (100.0, 1.6));
        for ms in [40, 60, 50] {
            quality.measured(Duration::from_millis(ms), Duration::from_millis(ms), 1000);
        }
        // 1 MB in 0.8 s past its head: 10 Mbps.
        quality.measured(
            Duration::from_millis(50),
            Duration::from_millis(850),
            1_000_000,
        );
        assert_eq!(quality.estimate(), (50.0, 10.0));
        // A middling body times neither.
        quality.measured(Duration::from_secs(9), Duration::from_secs(9), 10_000);
        assert_eq!(quality.estimate(), (50.0, 10.0));
    }

    #[test]
    fn a_page_is_told_the_estimate_capped_rounded_and_moved_by_its_hosts_factor() {
        for host in ["a.test", "b.test", "example.com", ""] {
            let factor = factor(host);
            assert!((0.9..=1.1).contains(&factor), "{factor}");
            assert_eq!(factor, super::factor(host), "one a host");
            let (kind, rtt, downlink) = told(host, (100.0, 1.6));
            assert_eq!(kind, "4g");
            assert!(
                [100.0, 50.0, 150.0].contains(&rtt) && rtt % 50.0 == 0.0,
                "{rtt}"
            );
            assert!((1.40..=1.80).contains(&downlink), "{downlink}");
            assert!(
                ((downlink * 1000.0).round() as u64).is_multiple_of(50),
                "{downlink}"
            );
            let (_, rtt, downlink) = told(host, (90_000.0, 900.0));
            assert_eq!((rtt, downlink), (3000.0, 10.0));
        }
        assert_eq!(effective_type(2500.0, 5.0), "slow-2g");
        assert_eq!(effective_type(1500.0, 5.0), "2g");
        assert_eq!(effective_type(300.0, 5.0), "3g");
        assert_eq!(effective_type(100.0, 0.5), "3g");
        assert_eq!(effective_type(100.0, 1.6), "4g");
    }
}
