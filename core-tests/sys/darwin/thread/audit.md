# `core.sys.darwin.thread` — implementation audit

## Status: **partial** (error-type + ONCE_* surface complete; live thread spawn deferred)

* DarwinThreadError 10-variant + Eq/Display/Debug pinned.
* ONCE_INIT / ONCE_RUNNING / ONCE_COMPLETE lifecycle constants pinned.
* Live `spawn` + `current_thread` + `Mutex`/`Condvar`/`SpinLock`/`Once`
  primitives need a real-thread fixture — VCS specs domain.

## Action items landed

1. `unit_test.vr` — 14 `@test`s: ONCE_INIT/RUNNING/COMPLETE values +
   pairwise distinctness; DarwinThreadError variant construction over
   5 payload + 3 unit variants; Eq reflexivity / payload sensitivity /
   variant disjointness.
2. `property_test.vr` — 5 laws: ONCE_* form partition of {0,1,2};
   ONCE_* pairwise distinct; Eq reflexive over unit + payload variants;
   payload-code sensitivity sweep; variant-disjoint Eq with symmetry pin.
3. `regression_test.vr` — 3 `@test`s: ONCE_INIT=0 zero-fill contract;
   monotone-increasing lifecycle; Eq payload-sensitive defect-class pin.

## Deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Live thread.spawn round-trip | Needs real thread; VCS specs domain. |
| 2 | Mutex/Condvar/SpinLock primitives | Live exercise needs concurrent contexts. |
| 3 | __ulock_wait / __ulock_wake (Darwin futex equivalent) | Requires controlled wakeup test harness. |
