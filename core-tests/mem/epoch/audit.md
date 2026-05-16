# `core.mem.epoch` — audit findings

> Module under test: `core/mem/epoch.vr` (674 LOC; 2 constants, 2
> callback function types, 1 record `EpochManager` with global
> instance `GLOBAL_EPOCH`, 1 record `EpochCache`, 6 free functions
> `current_epoch` / `cached_epoch` / `invalidate_epoch_cache` /
> `register_*_callback` / `unregister_*_callback` / `wraparound_count`).
>
> Test surfaces (this branch):
> `unit_test.vr` (~115 LOC), `property_test.vr` (~95 LOC),
> `integration_test.vr` (~75 LOC), `regression_test.vr` (~80 LOC).
>
> Tests pin the constants + the read-only surface (current_epoch,
> wraparound_count, cached_epoch).  Write-surface tests (epoch advance,
> callback round-trip) are not covered here because they require
> deliberate global-state manipulation that risks test-ordering
> non-determinism.

## 1. Cross-stdlib usage

`core.mem.epoch` is consumed by:

| Consumer | Use |
|---|---|
| `core/mem/header.vr` | `AllocationHeader.epoch_and_caps` low-16 bits store the epoch at allocation time. |
| `core/mem/thin_ref.vr` / `fat_ref.vr` | Each reference packs the current epoch into its `epoch_and_caps` field. |
| `core/base/memory.vr` | `Heap.new` / `Shared.new` call `current_epoch()` at construction time. |
| CBGR validation path | Every dereference compares the reference's stored epoch against the header's epoch. |

## 2. Crate-side hardcodes

Drift surfaces:

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `EPOCH_MAX = 0xFFFF` | 16-bit field width | Rust-side `verum_cbgr` mirror must agree; bit-shifted form in `pack_epoch_caps` writes the low 16 bits — if EPOCH_MAX widens past UInt16 the packing overlaps capability bits. |
| `DEFAULT_SYNC_INTERVAL = 1000` | Cache resync window | Performance tuning constant; not a soundness concern. |
| `GLOBAL_EPOCH` is `static mut` | Process-wide singleton | Multi-threaded access requires atomic operations; pinned via the `atomic_*_u64` intrinsic calls inside `current_epoch` / `wraparound_count`. |

## 3. Language-implementation gaps

### 3.1 `static mut GLOBAL_EPOCH` initialiser

The global is `public static mut GLOBAL_EPOCH: EpochManager = EpochManager { ... }`.
The initialiser fills every field with a zero / default value. This is
fine for the value-level shape but means a fresh `verum test` process
starts with `wraparound_count() == 0`. Tests that observe this must
not also FORCE a wraparound, or the assertion becomes order-dependent.

Pinned by `unit_test.test_wraparound_count_initially_zero_on_fresh_session`
(permissive — accepts ≥ 0).

### 3.2 Epoch advance triggering is not test-deterministic

`current_epoch()` reads via `atomic_load_u64`; an actual epoch
advance happens via `atomic_fetch_add_u64` from a writer site. No
test in this suite triggers an advance because the timing depends on
external bootkeeping (wraparound on 2^32 allocations or explicit
manual advance). Future test coverage for `register_epoch_callback`
round-trip will need a controlled-advance API.

### 3.3 Callback registration ID determinism

`register_epoch_callback(cb) -> Int` returns a unique ID that the
caller passes to `unregister_epoch_callback(id) -> Bool`. The IDs
are monotonically incremented across the process lifetime — they're
NOT reused after `unregister`. Two parallel tests calling
`register_epoch_callback` see different IDs.

This is a write-surface defect-risk surface that this branch does
NOT exercise. Deferred to a future cycle.

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| 1 | Missing test coverage for `core/mem/epoch.vr` | `core-tests/mem/epoch/{unit,property,integration,regression}_test.vr` | New 4-file suite; ~365 LOC total.  Read-only surface covered; write-surface deferred. |
| 2 | Missing `audit.md` for `core-tests/mem/epoch/` | This file. |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Test register/unregister callback round-trip with a fence between tests to isolate from other tests' callbacks. | ~45 min | open |
| §B | Test epoch advance: explicit `advance` API needed for test determinism; without one, the test would have to allocate 2^32 objects. | ~2 hours (requires stdlib API addition) | open |
| §C | Test wraparound: when epoch == EPOCH_MAX, the next advance must wrap to 0 AND increment wraparound_count. | Blocked on §B | open |
| §D | Cross-tier divergence sweep on `--aot` + `--interp`. | 1 hour wall-clock | open |
