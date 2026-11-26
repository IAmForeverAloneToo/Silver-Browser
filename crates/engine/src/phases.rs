//! Where a renderer's thread spends its time, for the site sweep
//! (2026-11-26): parse, script, style, layout and paint, each counted
//! once. A
//! phase entered inside another pauses it, so a layout a script forces is
//! layout's time, not the script's. Per thread: a renderer's engine runs
//! on one, which the sweep asks. Timed only in a renderer the sweep
//! started ([`time`]): elsewhere a phase costs one relaxed load.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// What a thread spends its time in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Parse,
    Script,
    Style,
    Layout,
    Paint,
}

/// The phases' names, in [`Phase`]'s order.
pub(crate) const PHASES: [&str; 5] = ["parse", "script", "style", "layout", "paint"];

#[derive(Default)]
struct Clock {
    /// The phases entered and not left, innermost last, each with when it
    /// last ran on.
    open: Vec<(Phase, Instant)>,
    spent: [Duration; PHASES.len()],
}

thread_local! {
    static CLOCK: RefCell<Clock> = RefCell::default();
}

/// Whether phases are timed in this process.
static TIMED: AtomicBool = AtomicBool::new(false);

/// Times the phases in this process from now on: the sweep's renderers.
pub(crate) fn time() {
    TIMED.store(true, Ordering::Relaxed);
}

/// Time in `phase` until the guard is dropped; the phase it was entered
/// in paused meanwhile.
#[must_use]
pub(crate) fn enter(phase: Phase) -> Timed {
    if !TIMED.load(Ordering::Relaxed) {
        return Timed(false);
    }
    let now = Instant::now();
    CLOCK.with(|clock| {
        let mut clock = clock.borrow_mut();
        if let Some(&(outer, since)) = clock.open.last() {
            clock.spent[outer as usize] += now - since;
        }
        clock.open.push((phase, now));
    });
    Timed(true)
}

/// A phase's time, counted until it is dropped (when it is timed).
pub(crate) struct Timed(bool);

impl Drop for Timed {
    fn drop(&mut self) {
        if !self.0 {
            return;
        }
        let now = Instant::now();
        CLOCK.with(|clock| {
            let mut clock = clock.borrow_mut();
            if let Some((phase, since)) = clock.open.pop() {
                clock.spent[phase as usize] += now - since;
            }
            // The outer phase runs on from now.
            if let Some(last) = clock.open.last_mut() {
                last.1 = now;
            }
        });
    }
}

/// The time spent in each phase on this thread so far, in milliseconds.
pub(crate) fn spent() -> [f64; PHASES.len()] {
    CLOCK.with(|clock| clock.borrow().spent.map(|d| d.as_secs_f64() * 1000.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phase_inside_another_is_counted_as_its_own() {
        time();
        let before = spent();
        let began = Instant::now();
        {
            let _script = enter(Phase::Script);
            std::thread::sleep(Duration::from_millis(20));
            {
                let _layout = enter(Phase::Layout);
                std::thread::sleep(Duration::from_millis(40));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let total = began.elapsed().as_secs_f64() * 1000.0;
        let after = spent();
        let script = after[Phase::Script as usize] - before[Phase::Script as usize];
        let layout = after[Phase::Layout as usize] - before[Phase::Layout as usize];
        // Each at least its sleeps, however long a runner sleeps; and
        // the two together the whole, so that the layout inside the
        // script is not the script's too (it would be 40 ms more).
        assert!(
            script >= 40.0 && layout >= 40.0,
            "script {script}, layout {layout} ms"
        );
        assert!(
            (script + layout - total).abs() < 10.0,
            "script {script} + layout {layout} ms, the whole {total} ms"
        );
    }
}
