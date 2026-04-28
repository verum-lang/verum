//! PermissionRouter warm-path microbench (#12 / P3.2).
//!
//! Verifies the architectural ≤2ns claim documented in
//! `interpreter/permission.rs`: a repeated request for the same
//! `(scope, target_id)` pair must hit the one-entry warm-path
//! cache with a single equality compare + branch.
//!
//! ## What we measure
//!
//! * `warm_path_allow_all`     — no policy, every check is a
//!   trivial `last == request` compare. The lower bound on
//!   what the gating system can ever cost.
//! * `warm_path_with_policy`   — policy installed, cache hits
//!   on every iteration. Same warm-path budget — the policy
//!   closure is never invoked.
//! * `cold_then_warm_per_pair` — alternating two distinct
//!   targets so `last` thrashes; the backing HashMap takes
//!   over and shows the +HashMap-probe overhead.
//! * `policy_cold_path`        — clear cache between every
//!   call, forcing the policy callback every time. This is
//!   the upper bound — represents pathological patterns that
//!   never benefit from caching.
//!
//! The interesting comparison is `warm_path_allow_all` vs
//! `cold_then_warm_per_pair`: the gap is what the warm-path
//! cache buys you when callers don't thrash.
//!
//! Run with:
//!
//! ```text
//! cargo bench -p verum_vbc --bench permission_router_bench
//! ```

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use verum_vbc::interpreter::permission::{
    PermissionDecision, PermissionRouter, PermissionScope,
};

fn warm_path_allow_all(c: &mut Criterion) {
    let mut group = c.benchmark_group("permission_router/warm_path");
    group.throughput(Throughput::Elements(1));
    group.bench_function("allow_all_repeat_same_target", |b| {
        let mut router = PermissionRouter::allow_all();
        // Prime the warm-path cache so the first iteration is
        // already on the fast path — we want to measure the
        // hot loop, not the cold-prime cost.
        let _ = router.check(PermissionScope::Syscall, 42);
        b.iter(|| {
            let decision = router.check(
                black_box(PermissionScope::Syscall),
                black_box(42),
            );
            black_box(decision);
        });
    });
    group.finish();
}

fn warm_path_with_policy(c: &mut Criterion) {
    let mut group = c.benchmark_group("permission_router/warm_path");
    group.throughput(Throughput::Elements(1));
    group.bench_function("with_policy_repeat_same_target", |b| {
        let mut router = PermissionRouter::with_policy(|_, _| PermissionDecision::Allow);
        let _ = router.check(PermissionScope::Network, 80);
        b.iter(|| {
            let decision = router.check(
                black_box(PermissionScope::Network),
                black_box(80),
            );
            black_box(decision);
        });
    });
    group.finish();
}

fn cold_then_warm_per_pair(c: &mut Criterion) {
    let mut group = c.benchmark_group("permission_router/thrash");
    group.throughput(Throughput::Elements(2));
    group.bench_function("alternating_two_targets", |b| {
        let mut router = PermissionRouter::with_policy(|_, _| PermissionDecision::Allow);
        // Prime both targets so the per-pair cost is purely
        // cache-lookup, not policy-invocation.
        let _ = router.check(PermissionScope::Syscall, 1);
        let _ = router.check(PermissionScope::Syscall, 2);
        b.iter(|| {
            let a = router.check(
                black_box(PermissionScope::Syscall),
                black_box(1),
            );
            let b_ = router.check(
                black_box(PermissionScope::Syscall),
                black_box(2),
            );
            black_box((a, b_));
        });
    });
    group.finish();
}

fn policy_cold_path(c: &mut Criterion) {
    let mut group = c.benchmark_group("permission_router/cold_path");
    group.throughput(Throughput::Elements(1));
    group.bench_function("policy_invoked_every_time", |b| {
        let mut router = PermissionRouter::with_policy(|_, _| PermissionDecision::Allow);
        b.iter(|| {
            // Clear cache each iteration so the policy is
            // invoked on every check — represents the upper
            // bound on per-check cost.
            router.clear_cache();
            let decision = router.check(
                black_box(PermissionScope::FileSystem),
                black_box(0xCAFE_BABE),
            );
            black_box(decision);
        });
    });
    group.finish();
}

fn deny_path(c: &mut Criterion) {
    let mut group = c.benchmark_group("permission_router/deny");
    group.throughput(Throughput::Elements(1));
    group.bench_function("cached_deny_short_circuit", |b| {
        let mut router = PermissionRouter::with_policy(|_, _| PermissionDecision::Deny);
        // Prime the deny decision into the warm-path cache.
        let _ = router.check(PermissionScope::Process, 0xDEAD);
        b.iter(|| {
            let decision = router.check(
                black_box(PermissionScope::Process),
                black_box(0xDEAD),
            );
            black_box(decision);
        });
    });
    group.finish();
}

criterion_group!(
    permission_benches,
    warm_path_allow_all,
    warm_path_with_policy,
    cold_then_warm_per_pair,
    policy_cold_path,
    deny_path,
);
criterion_main!(permission_benches);
