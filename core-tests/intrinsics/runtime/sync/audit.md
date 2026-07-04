# `intrinsics/runtime/sync` audit

Module: `core/intrinsics/runtime/sync.vr` (~153 LOC) — futex, spinlock,
CPU hints/fences, waitgroup (#65).

Tests: unit (9) + property (3) + regression (1) — single-threaded
value-level: spinlock state machine over a live List-backed UInt32 cell,
futex mismatch/no-waiter edges, hint/fence smoke, waitgroup counter
algebra (add(n) ⇔ n×done).  Inter-thread semantics (contention, wake
ordering, timeout-under-wait) belong to the concurrency suite — a
single-threaded runner can only pin these edges.

## Resolution (2026-07-04, follow-up batch)

SYNC-TLS-WIRING-1 FIXED on the interpreter — suite 13/13 interp:

* spinlock trio → dedicated `SystemSubOpcode` 0xB3-0xB5 (AtomicU32
  cmpxchg / release store / load-compare) replacing the shape-misused
  `OpcodeWithSize(AtomicCas/AtomicStore/AtomicLoad)` strategies whose
  missing operands made try_lock always-false.
* memory_fence → `FenceSeq` emitting `AtomicFence {{ ordering: 5 }}` —
  the DirectOpcode route emitted the opcode WITHOUT its ordering byte
  (truncated instruction, InvalidBytecode at the call).  compiler_fence
  and spin_hint (`OpcodeWithMode(AtomicFence, 0xFF)` — same truncation)
  are Tier-0 no-ops by contract.
* futex_wait: the test's `>= -1` assertion was a wrong guess — the ABI
  (handlers/ffi_extended.rs) returns -EAGAIN (-11) on value mismatch.
* RAWPTR-DROPREF-1 (found by the unpin sweep): `Value::from_ptr` interior
  addresses (as_mut_ptr results, PtrAdd/PtrSub outputs) are treated as
  droppable heap objects — DropRef at scope exit reinterprets ELEMENT
  BYTES as an ObjectHeader (`law_fetch_xor`'s stored 0xF0 crashed; other
  values happened not to).  Raw addresses are now INT-tagged end to end;
  every consumer reads through the dual int-or-pointer extraction.

AOT leg (documented residuals, task #5): try_lock/unlock/reacquire GREEN
via the new cmpxchg arms; waitgroup family needs a real AOT runtime (the
interp route is name-dispatch); futex return-convention must be pinned
cross-tier (-11 contract); is_locked arm + tls_get_base arm small gaps.

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
