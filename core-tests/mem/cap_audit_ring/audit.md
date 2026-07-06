# `core.mem.cap_audit_ring` — audit findings

> Module under test: `core/mem/cap_audit_ring.vr` (577 LOC; 1 constant
> `CAP_AUDIT_RING_CAPACITY`, 4 read-side functions (`recent`, `count`,
> `is_enabled`, accessor of the ring), 2 toggle functions (`enable` /
> `disable`), 1 test-only reset (`reset_for_tests`), 6 producer-side
> writers (`record_revoke` / `_attenuate` / `_ref_incr` / `_ref_decr` /
> `_gen_bump` / `_epoch_advance`), 1 raw `commit(event)` entry point).
>
> Test surfaces (this branch):
> `unit_test.vr` (~165 LOC), `property_test.vr` (~120 LOC),
> `integration_test.vr` (~125 LOC), `regression_test.vr` (~100 LOC).
>
> Tests exercise the public single-producer surface — the SPMC race
> properties (multi-producer ordering, seqlock-retry contention) are
> NOT exercised by these tests because verum_vbc's test runner has no
> spawn primitive that lets a test fork into multiple writer threads.

## 1. Cross-stdlib usage

| Consumer | Use |
|---|---|
| `core/mem/header.vr` | Imports `record_revoke`, `record_attenuate`, `record_ref_incr`, `record_ref_decr`, `record_gen_bump` and calls them at the CBGR writer entry points. |
| `core/mem/cap_audit.vr` | Provides the `CapEvent` payload type the ring stores. |
| Future panic handler | Calls `recent(N)` to grab the most-recent events for post-mortem context. |

## 2. Crate-side hardcodes

Drift surfaces:

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `CAP_AUDIT_RING_CAPACITY = 256` | Slot count + power-of-2 mod arithmetic | Drift would force a non-power-of-2 division (expensive); also affects observers that hardcode the window size. |
| Seqlock layout: `state` UInt64 carrying (writing_flag << 63, seq) | Concurrency protocol | Reader's `state1 == state2 == seq` check assumes this exact bit-layout. Drift = silent torn reads. |
| `record_*` writer signatures (ptr_id / gen_before / gen_after / caps_before / caps_after / timestamp_ns) | Producer-side ABI | Every writer site in `header.vr` calls with these exact argument shapes. Drift = compile error. |

## 3. Language-implementation gaps

### 3.1 SPMC ring buffer is single-producer in test exposure

Each test in this branch makes serial `record_*` calls — the ring's
SPMC contention path is NOT exercised. Multi-producer test coverage
requires spawning multiple writer tasks, which currently isn't safely
testable from the conformance suite.

### 3.2 `reset_for_tests` is a test-only API

The function clears the ring's state for test isolation; it MUST NOT
be called from production code. Tests in this suite call it at the
start of each scenario.

### 3.3 Disable/enable interleave doesn't drop sequence numbers

Pre-fix some drafts incremented NEXT_SEQ before checking the
enabled-flag, which leaked sequence numbers across enable/disable
boundaries. Pinned by `regression_test §C`.

### 3.4 `recent(n)` defensive bounds

Negative `n` returns empty without panic. Pre-fix the bound
arithmetic could underflow. Pinned by `regression_test §D`.

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| 1 | Missing test coverage for `core/mem/cap_audit_ring.vr` | `core-tests/mem/cap_audit_ring/{unit,property,integration,regression}_test.vr` | New 4-file suite; ~510 LOC total. |
| 2 | Missing `audit.md` for `core-tests/mem/cap_audit_ring/` | This file. |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Multi-producer race tests — requires spawn primitive in the test runner. | Blocked on test infrastructure | open |
| §B | Cross-tier divergence sweep on `--aot` + `--interp`. | 1 hour wall-clock | open |
| §C | Test the seqlock retry loop — requires deliberate writer-contention timing. | Blocked on §A | open |
| §D | Test `commit(event)` direct entry point (bypassing record_* writers). | ~30 min | open |

## 6. Session 2026-07-05 — live ring closed; 36/36 GREEN

The 9 live-path failures (`commit` / `recent` / `record_*` family,
formerly the InvalidOpcode-29 PC-desync, latterly a clean
"field write out of bounds: field index 4 … type='CapAuditSlot'")
root-caused to TWO fundamental defects, both closed in `799cff9b2`:

1. **CAP-AUDIT-SLOT-LAYOUT-1 / TYPE-NAME-INFERENCE-1** — the codegen
   Cast arm resolved only single-ident Path targets, so
   `let slot_ptr = &mut CAP_AUDIT_RING[idx] as *mut CapAuditSlot`
   carried NO type; `(*slot_ptr).event = stamped` then resolved the
   field with `type_name=None` and the global-intern fallback produced
   slot index 4 for a 2-field record (proven via
   `VERUM_TRACE_FIELDSHIFT`: `resolve('None','event') = 4 fn=commit`).
   Cast now routes through `extract_type_name_from_ast` (all type
   shapes incl. raw pointers) and the Deref arms strip
   `*mut `/`*const `/`*volatile ` carriers.

2. **ATOMIC-CAS-ZEROINIT-1** — 8-byte atomic CAS compares NaN-boxed
   bit patterns, but a freshly allocated static-mut cell is RAW zero:
   `expected == 0` could never match, so `NEXT_SEQ`'s inlined
   fetch_add (`AtomicLoad + Add + AtomicCas`) silently lost every
   increment — `commit` returned seq=1 forever and `count()` stayed 0.
   The CAS now accepts the raw-zero never-stored pattern when
   `expected == 0` (single retry; contention-correct).

Round-trip proven end-to-end: two `record_revoke` commits → seqs 1,2;
`count()==2`; `recent(2)` returns both events with correct seq/ptr_id.
Suite: **36/36 GREEN** (was 27/9).  Deferred items §A/§C (SPMC race
coverage) remain open pending a task-spawn primitive.

## 7. Session 2026-07-06 — AOT 34/36 (was 27/9)

DEREF-INTERIOR-1 + CLONE-AOT-ALIAS-1 (commit `53e13bf29`): the AOT
`Deref` of a `RefListElement` result double-dereferenced (the slot
record's zero header word treated as an address — `commit`
EXC_BAD_ACCESS 0x18), and AOT `Clone` pointer-aliased the repeat-array
initializer so all 256 ring "slots" were ONE record.  With interior
marks passing through `Deref`/`Mov` and Clone emitting a real
`checked_malloc+memcpy` copy, the live commit/count/recent round-trip
runs under `--aot`.  2 residual: `recent()` skips slot 0 in the
2-commit window (n=1-vs-2 cross-tier detail) — pinned, minimal repro
`repro_ring2.vr` in the session scratchpad.
