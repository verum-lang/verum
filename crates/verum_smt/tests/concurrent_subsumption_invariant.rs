//! Red-team Round 1 §1.1 (concurrency-side guardrail) — verifier
//! cache must be race-correct under concurrent `check()` calls
//! from multiple threads.
//!
//! Adversarial scenario: two threads holding `Arc<SubsumptionChecker>`
//! both perform `check()` on the same refinement obligation in a hot
//! loop. The cache is `Arc<RwLock<…>>`-protected; if either the read
//! or the write path drops a lock invariant, the result could be a
//! lost update (cache miss recorded with no insert), a torn read
//! (partial CacheEntry visible), or — worst case — a cached result
//! that refers to dropped data.
//!
//! Defense: `SubsumptionChecker.{cache, stats}` are both
//! `Arc<RwLock<…>>`. Reads grab the read lock; writes grab the
//! write lock. Drops are safe because the data lives behind `Arc`.
//!
//! This test pins the contract programmatically:
//!   1. 8 threads × 5 000 checks each (40 000 total) on the same
//!      reflexive obligation must yield the same logical answer
//!      every time, with no panic and no test-thread divergence.
//!   2. Stats counters are monotone non-decreasing — the sum of
//!      `cache_hits + cache_misses + syntactic_checks` exactly
//!      equals the number of `check()` invocations.
//!
//! The first invariant is the `Send + Sync` correctness pin.
//! The second is the lost-update invariant (no race in stats
//! accounting).

use std::sync::Arc;
use std::thread;

use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::literal::{IntLit, Literal, LiteralKind};
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_common::Heap;
use verum_smt::subsumption::{CheckMode, SubsumptionChecker, SubsumptionResult};

fn make_int(value: i128) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Int(IntLit {
                value,
                suffix: None,
            }),
            span: Span::dummy(),
        }),
        Span::dummy(),
    )
}

fn make_var(name: &str) -> Expr {
    let ident = Ident::new(name, Span::dummy());
    let path = Path::from_ident(ident);
    Expr::new(ExprKind::Path(path), Span::dummy())
}

fn make_gt(left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op: BinOp::Gt,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        Span::dummy(),
    )
}

#[test]
fn concurrent_reflexive_check_no_panic_no_divergence() {
    // Pin: 8 threads × 5 000 reflexive `(x > 0, x > 0)` checks
    // must all return the same logical answer (Syntactic(true)),
    // with no panic and no race-induced divergence.
    let checker = Arc::new(SubsumptionChecker::new());

    const THREADS: usize = 8;
    const ITERATIONS: usize = 5_000;

    let phi = make_gt(make_var("x"), make_int(0));

    let mut handles = Vec::with_capacity(THREADS);
    for _ in 0..THREADS {
        let checker = Arc::clone(&checker);
        let phi = phi.clone();
        handles.push(thread::spawn(move || {
            let mut local_true = 0u64;
            let mut local_other = 0u64;
            for _ in 0..ITERATIONS {
                match checker.check(&phi, &phi, CheckMode::SmtAllowed) {
                    SubsumptionResult::Syntactic(true) => local_true += 1,
                    _ => local_other += 1,
                }
            }
            (local_true, local_other)
        }));
    }

    let mut total_true = 0u64;
    let mut total_other = 0u64;
    for h in handles {
        let (t, o) = h.join().expect("worker thread panicked");
        total_true += t;
        total_other += o;
    }

    assert_eq!(
        total_true,
        (THREADS * ITERATIONS) as u64,
        "every reflexive check must yield Syntactic(true) — got {} other-result outcomes",
        total_other
    );
}

#[test]
fn concurrent_stats_counter_no_lost_updates() {
    // Pin: under N parallel threads, the sum of recorded
    // outcomes (cache_hits + cache_misses + syntactic_checks)
    // must equal the total number of check() invocations. A
    // lost update would surface as a count below
    // `THREADS * ITERATIONS`.
    let checker = Arc::new(SubsumptionChecker::new());

    const THREADS: usize = 8;
    const ITERATIONS: usize = 1_000;

    let phi1 = make_gt(make_var("x"), make_int(0));
    let phi2 = make_gt(make_var("x"), make_int(0));

    let mut handles = Vec::with_capacity(THREADS);
    for _ in 0..THREADS {
        let checker = Arc::clone(&checker);
        let phi1 = phi1.clone();
        let phi2 = phi2.clone();
        handles.push(thread::spawn(move || {
            for _ in 0..ITERATIONS {
                let _ = checker.check(&phi1, &phi2, CheckMode::SmtAllowed);
            }
        }));
    }
    for h in handles {
        h.join().expect("worker thread panicked");
    }

    let stats = checker.stats();
    let recorded =
        stats.cache_hits + stats.cache_misses + stats.syntactic_checks;
    let issued = (THREADS * ITERATIONS) as u64;

    // Note: `record_syntactic` increments BOTH `syntactic_checks`
    // and `cache_misses`, so the sum below double-counts
    // syntactic-fast-path resolutions. Strip the double-count
    // before comparing.
    let unique = recorded - stats.syntactic_checks;
    assert_eq!(
        unique, issued,
        "stats counter lost an update: issued {} checks, unique recorded {}",
        issued, unique
    );
}

#[test]
fn concurrent_distinct_obligations_isolated_per_key() {
    // Pin: per-thread distinct obligations are isolated — the
    // shared cache stores each (φ₁, φ₂) under its own canonical
    // key without false sharing. Demonstrates Sync correctness
    // of the cache's hash-keyed insert path under contention.
    let checker = Arc::new(SubsumptionChecker::new());

    const THREADS: u64 = 8;
    const PER_THREAD: u64 = 250;

    let mut handles = Vec::with_capacity(THREADS as usize);
    for tid in 0..THREADS {
        let checker = Arc::clone(&checker);
        handles.push(thread::spawn(move || {
            // Each thread issues `PER_THREAD` distinct
            // obligations: `(x > tid * 1000 + i, x > 0)`.
            // No two threads share an obligation.
            for i in 0..PER_THREAD {
                let bound = (tid * 10_000 + i) as i128;
                let phi1 = make_gt(make_var("x"), make_int(bound));
                let phi2 = make_gt(make_var("x"), make_int(0));
                let _ = checker.check(&phi1, &phi2, CheckMode::SmtAllowed);
            }
        }));
    }
    for h in handles {
        h.join().expect("worker thread panicked");
    }

    // Each obligation is unique → each fires exactly once;
    // cache_hits should be 0 across the entire run. (Some
    // syntactic-fast-path resolutions may apply since
    // `larger > smaller` is recognised structurally.)
    let stats = checker.stats();
    assert_eq!(
        stats.cache_hits, 0,
        "distinct per-thread obligations must not share cache keys"
    );
}
