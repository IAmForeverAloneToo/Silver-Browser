//! The hooks' timing (G247): how often each hook gap batch H added to
//! the engine's paths runs, and for how long, to say what the batch
//! costs a page that uses none of its features. Built only with the
//! `hook-timing` feature, which every crate with a hook has and passes
//! down to this one; without it [`hook!`](crate::hook) is nothing.
//!
//! A hook is a site: [`hook!`](crate::hook) at the top of a block makes
//! a guard that counts a call and adds the time until the block ends.
//! A hook inside another counts in both. The sites are kept as they are
//! first entered; [`take`] lists them, slowest first, and starts the
//! counts over, so that a benchmark reads one table a measurement.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, Once};
use std::time::Instant;

/// One hook's counts.
pub struct Site {
    name: &'static str,
    calls: AtomicU64,
    nanos: AtomicU64,
    listed: Once,
}

static SITES: Mutex<Vec<&'static Site>> = Mutex::new(Vec::new());

impl Site {
    /// A site named `name`, counting nothing yet.
    pub const fn new(name: &'static str) -> Site {
        Site {
            name,
            calls: AtomicU64::new(0),
            nanos: AtomicU64::new(0),
            listed: Once::new(),
        }
    }

    /// A call of the hook, timed until the guard is dropped.
    pub fn enter(&'static self) -> Guard {
        self.listed.call_once(|| {
            SITES
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(self);
        });
        Guard {
            site: self,
            start: Instant::now(),
        }
    }
}

/// A call being timed.
pub struct Guard {
    site: &'static Site,
    start: Instant,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let nanos = u64::try_from(self.start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.site.calls.fetch_add(1, Ordering::Relaxed);
        self.site.nanos.fetch_add(nanos, Ordering::Relaxed);
    }
}

/// The table since the last, headed `title`: each hook that ran, its
/// calls and milliseconds, slowest first; the counts start over.
pub fn take(title: &str) -> String {
    let sites = SITES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut rows: Vec<(u64, u64, &str)> = sites
        .iter()
        .map(|site| {
            (
                site.nanos.swap(0, Ordering::Relaxed),
                site.calls.swap(0, Ordering::Relaxed),
                site.name,
            )
        })
        .filter(|&(_, calls, _)| calls > 0)
        .collect();
    rows.sort_unstable_by(|a, b| b.cmp(a));
    let mut out = format!("hooks: {title}\n");
    for (nanos, calls, name) in rows {
        out.push_str(&format!(
            "  hook {name}: {calls} calls, {:.3} ms\n",
            nanos as f64 / 1e6
        ));
    }
    out
}
