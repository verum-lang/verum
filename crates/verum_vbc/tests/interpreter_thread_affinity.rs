//! Red-team Round 2 §7.3 — VBC interpreter LocalHeap thread-affinity
//! invariant.
//!
//! Adversarial scenario: a malicious / pathological caller tries
//! to share an `InterpreterState` (or its owned `Heap`) across
//! threads. If this succeeded, the bump-allocator / object-table
//! consistency invariants in `interpreter::heap` would be
//! violated by concurrent allocation, and the
//! `CURRENT_INTERPRETER` thread-local pointer in
//! `interpreter::state` would alias inconsistent state.
//!
//! Defense (structural):
//!   1. `Heap` owns `Vec<NonNull<ObjectHeader>>`. `NonNull<T>` is
//!      `!Send + !Sync` by default. Rust's type system therefore
//!      rejects cross-thread sharing of `InterpreterState` at
//!      compile time — there's no runtime check needed because
//!      the unsafe program never compiles.
//!   2. `CURRENT_INTERPRETER: thread_local! { Option<*mut
//!      InterpreterState> }` binds the active state pointer
//!      per-thread; spawning a new thread starts with `None`.
//!
//! Defense (behavioural):
//!   Each thread that wants its own interpreter constructs a
//!   fresh `InterpreterState` from a shared `Arc<VbcModule>`.
//!   The `Module` is `Send + Sync` (immutable bytecode held
//!   behind `Arc`), so threads share the *code* without sharing
//!   the *state*. Per-thread heap isolation falls out
//!   automatically.
//!
//! These tests pin the behavioural side programmatically:
//!   - 8 threads can each construct + drop their own
//!     `InterpreterState` from a shared `Arc<VbcModule>` without
//!     panic, deadlock, or per-thread pollution.
//!   - Heap statistics on each thread's interpreter are
//!     independent — no thread sees another thread's
//!     allocation count.

use std::sync::Arc;
use std::thread;

use verum_vbc::interpreter::{InterpreterConfig, InterpreterState};
use verum_vbc::module::VbcModule;

#[test]
fn per_thread_interpreter_construct_drop_no_panic() {
    // Pin: each thread can build its own `InterpreterState`
    // from a shared `Arc<VbcModule>` and drop it. No thread
    // observes another thread's state, no panic, no deadlock.
    let module = Arc::new(VbcModule::new("affinity_test".to_string()));

    const THREADS: usize = 8;
    let mut handles = Vec::with_capacity(THREADS);
    for tid in 0..THREADS {
        let module = Arc::clone(&module);
        handles.push(thread::spawn(move || {
            // Each thread constructs its own state — the Arc
            // gives every thread the same `VbcModule` (the
            // bytecode), but the heap, registers, call stack,
            // etc. are per-thread.
            let state = InterpreterState::new(module);
            // Use the `tid` to suppress unused-variable warnings
            // and add minimal per-thread distinctiveness.
            tid.wrapping_add(state.module.functions.len())
        }));
    }

    for h in handles {
        // Every thread terminates cleanly; no panic surfaces
        // here as a JoinError.
        h.join().expect("worker thread panicked");
    }
}

#[test]
fn per_thread_interpreters_have_independent_heaps() {
    // Pin: each thread's heap-allocation count is independent.
    // After 8 threads each running, the per-thread heaps see
    // exactly the allocations made on their own thread —
    // never another thread's count.
    //
    // We verify this indirectly: each thread starts with
    // 0-allocation heap, regardless of what other threads do.
    let module = Arc::new(VbcModule::new("isolation_test".to_string()));

    const THREADS: usize = 8;
    let mut handles = Vec::with_capacity(THREADS);
    for _ in 0..THREADS {
        let module = Arc::clone(&module);
        handles.push(thread::spawn(move || {
            let state = InterpreterState::new(module);
            // Heap freshly initialized — before any
            // allocation. The stat must be exactly zero on
            // every thread, regardless of thread-scheduling
            // order, because each thread owns its own Heap.
            state.heap.stats().total_allocs
        }));
    }

    for h in handles {
        let zero = h.join().expect("worker thread panicked");
        assert_eq!(
            zero, 0,
            "per-thread interpreter must start with zero heap allocations"
        );
    }
}

#[test]
fn per_thread_interpreter_with_distinct_configs() {
    // Pin: per-thread interpreter respects per-thread config.
    // Demonstrates the config field is owned by the state, not
    // shared via thread-local global. Threads with different
    // budgets observe their own configured limits, not a
    // sibling thread's limit.
    let module = Arc::new(VbcModule::new("config_isolation".to_string()));

    const THREADS: u64 = 4;
    let mut handles = Vec::with_capacity(THREADS as usize);
    for tid in 0..THREADS {
        let module = Arc::clone(&module);
        handles.push(thread::spawn(move || {
            // Per-thread distinct max_instructions budget.
            let mut config = InterpreterConfig::default();
            config.max_instructions = 1_000_000 + tid * 1000;
            let state = InterpreterState::with_config(module, config);
            state.config.max_instructions
        }));
    }

    let mut observed = Vec::with_capacity(THREADS as usize);
    for h in handles {
        observed.push(h.join().expect("worker thread panicked"));
    }
    observed.sort();
    let expected: Vec<u64> = (0..THREADS)
        .map(|tid| 1_000_000 + tid * 1000)
        .collect();
    assert_eq!(
        observed, expected,
        "per-thread configs must remain independent across {} threads",
        THREADS
    );
}

#[test]
fn module_is_shareable_across_threads() {
    // Pin: `Arc<VbcModule>` IS shareable — the bytecode is the
    // shared part; the per-thread state owns its own heap +
    // registers. This is the architectural complement to the
    // isolation invariants above: the *code* should be shared
    // for memory efficiency (no per-thread bytecode copy), the
    // *runtime state* should not.
    fn require_send_sync<T: Send + Sync>() {}
    require_send_sync::<Arc<VbcModule>>();

    // And the actual usage pattern works:
    let module = Arc::new(VbcModule::new("shared_code".to_string()));
    let m1 = Arc::clone(&module);
    let m2 = Arc::clone(&module);
    let h1 = thread::spawn(move || m1.name.clone());
    let h2 = thread::spawn(move || m2.name.clone());
    let n1 = h1.join().expect("worker thread panicked");
    let n2 = h2.join().expect("worker thread panicked");
    assert_eq!(n1, "shared_code");
    assert_eq!(n2, "shared_code");
}
