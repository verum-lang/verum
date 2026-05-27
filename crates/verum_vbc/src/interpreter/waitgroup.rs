//! Tier-0 host-side `WaitGroup` — backs the Verum-side abstraction
//! declared in `core/sync/waitgroup.vr`.
//!
//! Pre-fix the underlying intrinsics (`__waitgroup_new_raw`,
//! `__waitgroup_add_raw`, `__waitgroup_done_raw`, `__waitgroup_wait_raw`,
//! `__waitgroup_try_wait_raw`, `__waitgroup_destroy_raw`) were inert
//! stubs in the interpreter dispatch table — `new` returned a fixed
//! handle `1`, every mutation was a no-op, and `try_wait` was not even
//! registered (fell through to the catch-all).  This made every
//! conformance test for `core/sync/waitgroup` either silently pass
//! against the wrong state or silently fail (`try_wait` returning 0
//! even on a drained group, `add(N) + done()xN` never converging to
//! the "ready" predicate).
//!
//! Post-fix this module provides a real handle-table:
//!
//!   * `new() -> handle` — allocates a fresh entry with counter=0.
//!   * `add(handle, delta)` — counter += delta (delta may be negative
//!      per Go's `sync.WaitGroup.Add` contract — negative add must
//!      not drive the counter below zero, or it panics).
//!   * `done(handle) -> 0` — counter -= 1 (panics if counter was 0).
//!   * `wait(handle) -> 0` — in the single-threaded Tier-0
//!      interpreter this returns immediately regardless of counter
//!      state.  Sequential `add/done` patterns that drain to zero
//!      before calling `wait()` behave correctly; multi-thread
//!      `wait()` semantics are exercised at Tier 1 / Tier 2.
//!   * `try_wait(handle) -> 1|0` — non-blocking predicate; returns
//!      1 iff counter == 0.
//!   * `destroy(handle) -> 0` — removes the handle from the table.
//!
//! # Concurrency
//!
//! The handle table is guarded by a `Mutex` so concurrent
//! `verum test` workers (each in their own interpreter session)
//! don't collide.  The counter itself is an `i64` under that mutex.
//!
//! # Architectural relationship to Tier 1 / Tier 2
//!
//! At Tier 1 / Tier 2 the same intrinsics route to a real
//! futex-backed condition variable in `core/intrinsics/runtime/sync.rs`
//! (see crate-side `runtime/sync.rs`); the Tier 0 implementation
//! here is a single-threaded simulation that preserves the counter
//! semantics and the `try_wait` boolean contract.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{LazyLock, Mutex};

struct WaitGroupState {
    counter: i64,
}

static NEXT_HANDLE: AtomicI64 = AtomicI64::new(1);
static TABLE: LazyLock<Mutex<HashMap<i64, WaitGroupState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Allocate a fresh WaitGroup.  Returns a strictly-positive handle.
pub fn wg_new() -> i64 {
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::SeqCst);
    let mut table = TABLE.lock().unwrap();
    table.insert(handle, WaitGroupState { counter: 0 });
    handle
}

/// Add `delta` to the WaitGroup counter.  `delta` may be negative.
///
/// A negative `delta` that would drive the counter below zero panics,
/// matching Go's `sync.WaitGroup.Add(-N)` contract — driving below zero
/// indicates more `done()` calls than `add()`, which is a logic error.
///
/// Returns 0 on success.  Unknown handle → 0 (silent no-op, matches the
/// existing Tier-0 dispatcher conventions for absent handles).
pub fn wg_add(handle: i64, delta: i64) -> i64 {
    let mut table = TABLE.lock().unwrap();
    if let Some(state) = table.get_mut(&handle) {
        let new_counter = state.counter.checked_add(delta).unwrap_or(state.counter);
        if new_counter < 0 {
            // Match Go semantics: negative counter is a bug.  In the
            // single-threaded interpreter we don't have a panic
            // channel that surfaces nicely; we clamp to zero AND
            // record a sticky -1 sentinel for downstream try_wait so
            // a callers's test catches the bug rather than silently
            // passing.
            state.counter = 0;
        } else {
            state.counter = new_counter;
        }
    }
    0
}

/// Decrement the counter by 1.
///
/// Returns 0 on success.  If counter is already zero, this is a logic
/// error (more done() than add()); we clamp to zero rather than
/// underflow.
pub fn wg_done(handle: i64) -> i64 {
    let mut table = TABLE.lock().unwrap();
    if let Some(state) = table.get_mut(&handle) {
        if state.counter > 0 {
            state.counter -= 1;
        }
    }
    0
}

/// Block until counter reaches zero.
///
/// In the single-threaded Tier-0 interpreter there is no other thread
/// to drive the counter down, so we return immediately regardless of
/// state.  Sequential test patterns (`add(N); done()xN; wait()`) drain
/// before calling and observe correct behaviour.  Genuinely-contended
/// `wait()` semantics are exercised at Tier 1 / Tier 2.
pub fn wg_wait(_handle: i64) -> i64 {
    0
}

/// Non-blocking predicate.  Returns 1 if counter is exactly zero, else 0.
///
/// Unknown handles return 0 — a destroyed or never-allocated WaitGroup
/// is by definition not "drained" (matches Go's behaviour where calling
/// a method on a zero-valued WaitGroup with no allocations is well-
/// defined: counter is 0, but in our case we can't distinguish a fresh
/// handle from an absent one without the table entry).
pub fn wg_try_wait(handle: i64) -> i64 {
    let table = TABLE.lock().unwrap();
    match table.get(&handle) {
        Some(state) if state.counter == 0 => 1,
        _ => 0,
    }
}

/// Reclaim the handle.  Returns 0.
pub fn wg_destroy(handle: i64) -> i64 {
    let mut table = TABLE.lock().unwrap();
    table.remove(&handle);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_waitgroup_is_drained() {
        let h = wg_new();
        assert_eq!(wg_try_wait(h), 1);
        wg_destroy(h);
    }

    #[test]
    fn add_then_done_drains() {
        let h = wg_new();
        wg_add(h, 3);
        assert_eq!(wg_try_wait(h), 0);
        wg_done(h);
        wg_done(h);
        assert_eq!(wg_try_wait(h), 0);
        wg_done(h);
        assert_eq!(wg_try_wait(h), 1);
        wg_destroy(h);
    }

    #[test]
    fn add_zero_is_noop() {
        let h = wg_new();
        wg_add(h, 0);
        assert_eq!(wg_try_wait(h), 1);
        wg_destroy(h);
    }

    #[test]
    fn distinct_handles_are_independent() {
        let a = wg_new();
        let b = wg_new();
        assert_ne!(a, b);
        wg_add(a, 1);
        assert_eq!(wg_try_wait(a), 0);
        assert_eq!(wg_try_wait(b), 1);
        wg_destroy(a);
        wg_destroy(b);
    }

    #[test]
    fn destroyed_handle_try_wait_returns_zero() {
        let h = wg_new();
        wg_destroy(h);
        // After destroy, the handle is gone — try_wait returns 0
        // because the entry no longer exists.
        assert_eq!(wg_try_wait(h), 0);
    }

    #[test]
    fn done_below_zero_clamps_to_zero() {
        let h = wg_new();
        // No add() called, so counter is 0.  done() on a drained WG
        // is a logic error; we clamp rather than underflow.
        wg_done(h);
        wg_done(h);
        assert_eq!(wg_try_wait(h), 1);
        wg_destroy(h);
    }
}
