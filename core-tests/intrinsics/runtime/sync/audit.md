# `intrinsics/runtime/sync` audit

Module: `core/intrinsics/runtime/sync.vr` (~153 LOC) — futex, spinlock,
CPU hints/fences, waitgroup (#65).

Tests: unit (9) + property (3) + regression (1) — single-threaded
value-level: spinlock state machine over a live List-backed UInt32 cell,
futex mismatch/no-waiter edges, hint/fence smoke, waitgroup counter
algebra (add(n) ⇔ n×done).  Inter-thread semantics (contention, wake
ordering, timeout-under-wait) belong to the concurrency suite — a
single-threaded runner can only pin these edges.

## Findings (2026-07-03 first pass)

* The suite depends on LIST-ASPTR-HEADER-1 being fixed (d31878ee8) — the
  regression guard pins that the lock word is the ELEMENT, not the list
  header (a regressed as_mut_ptr would CAS the length field).
* `futex_wait` timeout-path NOT pinned: a real timed wait blocks the
  runner for its duration and the return-code convention (0 vs -1)
  differs per platform doc; deferred to the concurrency suite.
* `spin_hint`/`spin_loop_hint` share one intrinsic key ("spin_hint") — an
  alias pair, no drift risk.

## Crate-side drift surfaces

* `SystemSubOpcode::{FutexWait,FutexWake,SpinlockLock} (0xB0-0xB2)` +
  `verum_futex_*`/`verum_spinlock_*` AOT runtime helpers.
* Waitgroup handles are interpreter-table indices — the magic-word class
  of handle-validation hardening (script-engine 7f8120b8e) does not cover
  them yet; candidate follow-up.

## Action items

* Concurrency-suite integration (real threads) — deferred.
* Waitgroup handle validation hardening — deferred.
