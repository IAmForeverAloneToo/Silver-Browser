//! `--startup-trace` (M35.6): the time from the process's start to each
//! step of a cold start, and to the first frame of the blank tab with
//! its page drawn, printed on standard error against [`STARTUP_BUDGET`].

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// What a cold start may take, from the process's start to the first
/// frame of the blank tab with its page drawn (M35.6,
/// docs/perf-budgets.md).
pub(crate) const STARTUP_BUDGET: Duration = Duration::from_millis(300);

/// How old the process was when the trace began, and when that was.
struct Trace {
    age: Duration,
    at: Instant,
}

static TRACE: OnceLock<Trace> = OnceLock::new();

/// Whether the first frame, and the first with the page drawn, are said.
static FRAMED: AtomicBool = AtomicBool::new(false);
static PAGED: AtomicBool = AtomicBool::new(false);

/// Starts the trace: the process's age from the platform, or none where
/// it cannot say, the time before `main` then left out.
pub(crate) fn begin() {
    let at = Instant::now();
    let age = crate::platform::process_age().unwrap_or_default();
    let _ = TRACE.set(Trace { age, at });
}

/// The time since the process started, when traced.
fn since_start() -> Option<Duration> {
    TRACE.get().map(|trace| trace.age + trace.at.elapsed())
}

/// Says that `step` is done, when traced.
pub(crate) fn mark(step: &str) {
    if let Some(time) = since_start() {
        eprintln!("startup: {step} {}", millis(time));
    }
}

/// A frame is presented, with the page drawn or not: the first of each
/// is said, when traced.
pub(crate) fn presented(page: bool) {
    if TRACE.get().is_none() {
        return;
    }
    if !FRAMED.swap(true, Ordering::Relaxed) {
        mark("first frame");
    }
    if page
        && !PAGED.swap(true, Ordering::Relaxed)
        && let Some(time) = since_start()
    {
        eprintln!("startup: {}", page_frame(time));
    }
}

/// What the trace says of the first frame with the page drawn, shown
/// `time` after the process started.
fn page_frame(time: Duration) -> String {
    let within = if time <= STARTUP_BUDGET {
        "within"
    } else {
        "over"
    };
    format!(
        "page frame {} ({within} the budget of {})",
        millis(time),
        millis(STARTUP_BUDGET)
    )
}

fn millis(time: Duration) -> String {
    format!("{:.1}ms", time.as_secs_f64() * 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_time_is_in_milliseconds_to_a_tenth() {
        assert_eq!(millis(Duration::from_micros(182_340)), "182.3ms");
        assert_eq!(millis(STARTUP_BUDGET), "300.0ms");
    }

    #[test]
    fn the_page_frame_is_held_to_the_budget() {
        assert_eq!(
            page_frame(Duration::from_millis(300)),
            "page frame 300.0ms (within the budget of 300.0ms)"
        );
        assert_eq!(
            page_frame(Duration::from_micros(300_100)),
            "page frame 300.1ms (over the budget of 300.0ms)"
        );
    }

    #[test]
    fn the_process_ages_as_the_clock_goes_where_the_platform_says() {
        // Where it cannot say, the trace begins at `main`.
        let Some(age) = crate::platform::process_age() else {
            return;
        };
        assert!(age > Duration::ZERO && age < Duration::from_secs(3600));
        let began = Instant::now();
        std::thread::sleep(Duration::from_millis(100));
        let later = crate::platform::process_age().expect("read again");
        let aged = later.saturating_sub(age).as_secs_f64();
        let slept = began.elapsed().as_secs_f64();
        assert!(aged > slept * 0.5 && aged < slept * 2.0, "{aged} {slept}");
    }
}
