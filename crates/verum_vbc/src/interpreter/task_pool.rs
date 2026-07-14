//! Interpreter-side ThreadPool result table (POOL-INTERP-STUB-1).
//!

//! The Tier-0 interpreter is single-threaded by design (see
//! GENERATE-NATIVE-WORKER-RACE: even AOT test harnesses run
//! `threads=1`), so `core.runtime.pool.ThreadPool` semantics are
//! implemented EAGERLY: `__pool_submit_raw` executes the submitted
//! function synchronously via `call_function_sync` and parks the
//! result here; `__pool_await_raw` collects it.  This keeps the
//! observable contract identical to Tier-1 (`verum_pool_*` native
//! workers): every submitted task runs exactly once and `await()`
//! returns its result — only the interleaving differs, which the
//! language does not promise.
//!

//! Handles are 1-based; slot 0 is never handed out so a zeroed
//! register can't alias a live task.  Slots are freed on `take` and
//! reused via free-list, keeping the table bounded under
//! fire-and-forget submission (mirrors the stdlib-side
//! `PoolTaskHandle.drop` drain contract).

use std::cell::RefCell;

thread_local! {
    static RESULTS: RefCell<PoolTable> = RefCell::new(PoolTable::default());
}

#[derive(Default)]
struct PoolTable {
    slots: Vec<Option<i64>>,
    free: Vec<usize>,
}

/// Park a completed task result; returns its 1-based handle.
pub(crate) fn store(result: i64) -> i64 {
    RESULTS.with(|t| {
        let mut t = t.borrow_mut();
        let idx = match t.free.pop() {
            Some(i) => {
                t.slots[i] = Some(result);
                i
            }
            None => {
                t.slots.push(Some(result));
                t.slots.len() - 1
            }
        };
        (idx + 1) as i64
    })
}

/// Collect a parked result, freeing the slot.  Unknown / already-taken
/// handles return 0 — the same defensive default the native pool uses
/// for a bogus handle (never a crash: a corrupted handle from user
/// code must not take down the runtime).
pub(crate) fn take(handle: i64) -> i64 {
    if handle <= 0 {
        return 0;
    }
    let idx = (handle - 1) as usize;
    RESULTS.with(|t| {
        let mut t = t.borrow_mut();
        match t.slots.get_mut(idx).and_then(|s| s.take()) {
            Some(v) => {
                t.free.push(idx);
                v
            }
            None => 0,
        }
    })
}

// ---------------------------------------------------------------------------
// THREAD-EAGER-TIER0-1: parked pthread return VALUES.
//
// Unlike the pool table above (i64 task results), pthread start
// routines return a full NaN-boxed `Value` (`&unsafe Byte` — often a
// pointer).  Round-trip the raw bit-pattern so pointer tags survive.
// Handles are 1-based and disjoint from real pthread_t values only by
// context: they are only ever produced and consumed by the Tier-0
// pthread_create / pthread_join intercepts.
// ---------------------------------------------------------------------------

thread_local! {
    static THREAD_RETVALS: RefCell<PoolTable> = RefCell::new(PoolTable::default());
}

/// Park an eagerly-computed thread return value (raw Value bits);
/// returns the synthetic thread handle.
pub(crate) fn thread_store(ret_bits: i64) -> i64 {
    THREAD_RETVALS.with(|t| {
        let mut t = t.borrow_mut();
        let idx = match t.free.pop() {
            Some(i) => {
                t.slots[i] = Some(ret_bits);
                i
            }
            None => {
                t.slots.push(Some(ret_bits));
                t.slots.len() - 1
            }
        };
        (idx + 1) as i64
    })
}

/// Collect a parked thread return value by synthetic handle.
/// Unknown / drained handles return 0 bits (nil-equivalent).
pub(crate) fn thread_take(handle: i64) -> i64 {
    if handle <= 0 {
        return 0;
    }
    let idx = (handle - 1) as usize;
    THREAD_RETVALS.with(|t| {
        let mut t = t.borrow_mut();
        match t.slots.get_mut(idx).and_then(|s| s.take()) {
            Some(v) => {
                t.free.push(idx);
                v
            }
            None => 0,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_take_roundtrip() {
        let h = store(42);
        assert!(h >= 1);
        assert_eq!(take(h), 42);
        // second take: slot already drained
        assert_eq!(take(h), 0);
    }

    #[test]
    fn handles_are_independent() {
        let a = store(1);
        let b = store(2);
        assert_ne!(a, b);
        assert_eq!(take(b), 2);
        assert_eq!(take(a), 1);
    }

    #[test]
    fn slot_reuse_after_take() {
        let a = store(7);
        assert_eq!(take(a), 7);
        let b = store(9);
        // freed slot is reused — table stays bounded
        assert_eq!(a, b);
        assert_eq!(take(b), 9);
    }

    #[test]
    fn bogus_handles_are_zero() {
        assert_eq!(take(0), 0);
        assert_eq!(take(-5), 0);
        assert_eq!(take(1_000_000), 0);
    }
}
