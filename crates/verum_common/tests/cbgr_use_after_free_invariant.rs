//! Red-team Round 2 §7.2 (companion guardrail) — CBGR
//! generation-counter use-after-free invariant under multi-reader
//! contention.
//!
//! Adversarial scenario: a single writer invalidates an
//! allocation while N readers concurrently call `validate()` with
//! a stale generation. The fundamental contract: every reader
//! that calls `validate(expected_gen, expected_epoch)` must
//! observe one of:
//!
//!   - `Success` if the allocation is still live AND its
//!     `(generation, epoch)` matches the reader's expectation.
//!   - `ExpiredReference` if the allocation has been invalidated.
//!   - `GenerationMismatch` if the slot has been reused.
//!
//! Soundness invariant: NO reader may observe `Success` after a
//! writer has called `invalidate()` (modulo reordering — the
//! invalidate's `Release` store synchronises with the reader's
//! `Acquire` load, so any reader who observes `actual_gen ==
//! expected_gen` must have synchronised with the pre-invalidate
//! state).
//!
//! This is the verifier-level companion to the hazard-pointer
//! protocol implemented in `core/mem/hazard.vr`. The Verum
//! stdlib's hazard pointer relies on this CBGR-counter
//! correctness — without it, the higher-level protocol cannot
//! defend against use-after-free regardless of how the hazard
//! list is structured.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;

use verum_common::cbgr::{CbgrErrorCode, CbgrHeader};

#[test]
fn use_after_free_validate_after_invalidate() {
    // Pin: after a writer's `invalidate()`, every subsequent
    // reader call to `validate()` MUST return
    // `ExpiredReference` (generation cleared to
    // GEN_UNALLOCATED). Single-threaded baseline.
    let h = CbgrHeader::new(0);
    let original_gen = h.generation();
    assert_eq!(h.validate(original_gen, 0), CbgrErrorCode::Success);
    h.invalidate();
    assert_eq!(
        h.validate(original_gen, 0),
        CbgrErrorCode::ExpiredReference,
        "validate must fail after invalidate"
    );
    assert!(!h.is_valid(), "is_valid must reflect post-invalidate state");
}

#[test]
fn concurrent_readers_respect_invalidate_release_acquire() {
    // Pin: Release/Acquire synchronisation prevents the
    // use-after-free observation. N readers spin-validate;
    // when the writer calls invalidate(), every subsequent
    // reader observes the invalidation. No reader may report
    // `Success` after the test's `done` flag is set —
    // the writer sets `done` AFTER the invalidate, so any
    // reader observing `done == true` MUST also see the
    // invalidate (Release/Acquire on `done` chains the
    // happens-before).
    let h = Arc::new(CbgrHeader::new(0));
    let original_gen = h.generation();
    let done = Arc::new(AtomicBool::new(false));

    const READERS: usize = 8;
    let mut handles = Vec::with_capacity(READERS);
    let saw_post_invalidate_success = Arc::new(AtomicU64::new(0));

    for _ in 0..READERS {
        let h = Arc::clone(&h);
        let done = Arc::clone(&done);
        let counter = Arc::clone(&saw_post_invalidate_success);
        handles.push(thread::spawn(move || {
            // Spin until the writer invalidates. After done is
            // set, validate one more time and assert the result
            // is NOT Success.
            while !done.load(Ordering::Acquire) {
                // Pre-invalidate validation: should be Success
                // unless the writer is in the middle of the
                // invalidate. We don't assert here because the
                // race window is narrow but legitimate.
                let _ = h.validate(original_gen, 0);
            }

            // Post-invalidate path: any reader who has
            // observed `done = true` MUST also observe the
            // invalidate (Release on done + Acquire on done
            // pairs synchronise the prior invalidate). A
            // `Success` here would be a use-after-free.
            //
            // Loop a few times so transient scheduler effects
            // don't suppress the issue.
            for _ in 0..1000 {
                if h.validate(original_gen, 0) == CbgrErrorCode::Success {
                    counter.fetch_add(1, Ordering::Relaxed);
                }
            }
        }));
    }

    // Brief settle, then invalidate.
    thread::sleep(std::time::Duration::from_millis(10));
    h.invalidate();
    done.store(true, Ordering::Release);

    for h in handles {
        h.join().expect("worker thread panicked");
    }

    let count = saw_post_invalidate_success.load(Ordering::Relaxed);
    assert_eq!(
        count, 0,
        "{} reader observations of Success POST-invalidate — \
         use-after-free defense is broken",
        count
    );
}

#[test]
fn writer_invalidate_then_revalidate_reuse() {
    // Pin: simulating slot reuse — after invalidate, a fresh
    // `CbgrHeader::new(...)` represents the new occupant.
    // Readers holding the old generation must NOT match the
    // new occupant (different addresses; generation alone is
    // not the identity, but combined with the epoch in the
    // header, the reader's expectation cannot collide with a
    // fresh allocation's pair). This is the ABA-prevention
    // property.
    let old = CbgrHeader::new(0);
    let old_gen = old.generation();
    old.invalidate();

    // "Reuse" the slot by constructing a fresh header. In a
    // real allocator this would be in the same memory; here we
    // emulate the conceptual sequence.
    let new = CbgrHeader::new(0);
    let new_gen = new.generation();

    // The fresh header's generation matches GEN_INITIAL, the
    // same value the old one had at construction. A reader
    // holding `(old_gen, epoch=0)` would naively succeed
    // against `new` if generation alone were the identity.
    // The defence is structural: the reader holds a pointer
    // that addresses the OLD allocation, not the new one;
    // CbgrHeader::validate checks the actual header at the
    // pointer. We verify the generations match exactly so the
    // ABA-prevention has to come from the address, not the
    // generation alone — which is the documented architecture.
    assert_eq!(old_gen, new_gen, "fresh allocation reuses GEN_INITIAL");

    // The old header's validate post-invalidate fails as
    // expected:
    assert_eq!(
        old.validate(old_gen, 0),
        CbgrErrorCode::ExpiredReference,
        "old header must report ExpiredReference"
    );
    // The new header's validate succeeds with the same
    // generation/epoch pair:
    assert_eq!(
        new.validate(new_gen, 0),
        CbgrErrorCode::Success,
        "new header must report Success"
    );
}

#[test]
fn invalidate_is_idempotent_under_contention() {
    // Pin: multiple concurrent invalidate() calls must leave
    // the header in the invalidated state. No race-induced
    // partial state visible.
    let h = Arc::new(CbgrHeader::new(0));
    const THREADS: usize = 8;
    let mut handles = Vec::with_capacity(THREADS);
    for _ in 0..THREADS {
        let h = Arc::clone(&h);
        handles.push(thread::spawn(move || {
            for _ in 0..1000 {
                h.invalidate();
            }
        }));
    }
    for h in handles {
        h.join().expect("worker thread panicked");
    }
    // After all invalidate() calls, the header must report
    // not-valid.
    assert!(
        !h.is_valid(),
        "concurrent invalidate must leave header in invalidated state"
    );
    // And validate() must return ExpiredReference for any
    // generation.
    assert_eq!(
        h.validate(1, 0),
        CbgrErrorCode::ExpiredReference,
        "post-concurrent-invalidate validate must surface ExpiredReference"
    );
}
