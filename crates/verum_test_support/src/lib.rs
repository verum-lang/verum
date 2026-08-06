//! Shared measurement authority for wall-clock performance CONTRACTS in
//! integration tests (dev-dependency only; never ships in a release
//! artifact).
//!
//! The class this crate retires: a single cold-start `Instant::now()`
//! sample judged against a tight absolute threshold. On a shared CI
//! runner that measurement is dominated by scheduler noise — first-fault
//! page-ins, cold caches, sibling-VM steal — not by the algorithm under
//! test. The `kind_inference_tests` gate observed 926µs for an operation
//! whose warm median is single-digit microseconds, and failed the whole
//! Integration job on it.
//!
//! What a test-side performance contract can honestly pin is the
//! ALGORITHM (no accidental exponential/quadratic blowup), not absolute
//! latency on unknown hardware. The honest instrument for that is:
//! warm up once (populate lazy statics, fault pages in), sample several
//! times, judge the MEDIAN. A real complexity regression shifts every
//! sample; a scheduler spike shifts one or two, and the median ignores
//! them. Absolute-latency budgets belong in `benches/` (criterion),
//! where the harness owns warmup and statistics.

use std::time::{Duration, Instant};

/// Wall-clock median of `samples` runs of `op`, after one discarded
/// warmup run. Returns the median together with the LAST run's result so
/// callers can keep asserting correctness on the value; the result of
/// every run is passed through [`std::hint::black_box`] so the optimizer
/// cannot delete the measured work.
///
/// `samples` is clamped to at least 1.
pub fn median_elapsed<T>(samples: usize, mut op: impl FnMut() -> T) -> (Duration, T) {
    let samples = samples.max(1);
    // Warmup: the discarded run eats one-time costs (lazy statics,
    // allocator warm-up, page faults) that a contract about the
    // algorithm must not bill to it.
    let mut last = std::hint::black_box(op());
    let mut times = Vec::with_capacity(samples);
    for _ in 0..samples {
        let start = Instant::now();
        last = std::hint::black_box(op());
        times.push(start.elapsed());
    }
    (median(&mut times), last)
}

/// Median of `samples` durations REPORTED by `op` (for APIs that measure
/// themselves — solver results carrying a `duration` field). One
/// discarded warmup run, same rationale as [`median_elapsed`].
///
/// `samples` is clamped to at least 1.
pub fn median_reported(samples: usize, mut op: impl FnMut() -> Duration) -> Duration {
    let samples = samples.max(1);
    let _warmup = std::hint::black_box(op());
    let mut times: Vec<Duration> = (0..samples).map(|_| op()).collect();
    median(&mut times)
}

fn median(times: &mut [Duration]) -> Duration {
    times.sort_unstable();
    times[times.len() / 2]
}
