//! What the benchmarks share (M35.1, T4.2): the quick mode CI runs each
//! of them in, once on every push, to see that it still builds and
//! runs. Its numbers say nothing there; they are taken on the
//! maintainer's machine, by turns (T4.3, `docs/perf-budgets.md`).

/// Whether `SILVER_BENCH_QUICK` asks for the quick mode.
pub fn quick() -> bool {
    std::env::var_os("SILVER_BENCH_QUICK").is_some()
}

/// `n` rounds, or one in the quick mode.
#[allow(dead_code)]
pub fn rounds(n: usize) -> usize {
    if quick() { 1 } else { n }
}

/// `n` frames, or a tenth of them, at least one, in the quick mode.
#[allow(dead_code)]
pub fn frames(n: u32) -> u32 {
    if quick() { (n / 10).max(1) } else { n }
}
