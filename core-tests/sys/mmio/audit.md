# `core.sys.mmio` — implementation audit

## Status: **partial** (type-level surface covered; runtime MMIO deferred)

* Type-level API (`BarrierKind`, `MemoryFlags`, `MemoryRegion`,
  `AccessMode` union, `compiler_barrier`, `dmb`) is covered by
  `unit_test.vr` and pinned in `regression_test.vr`.
* `MmioRegister<T, MODE>` (the generic `*volatile mut T` wrapper),
  `VerifiedRegister<T, MODE>` (the ghost-state-tracked variant), and
  the `RegisterBlock` protocol are **not exercised at runtime** —
  they require a real MMIO page (hardware register or shared-memory
  fixture), which is out-of-scope for the conformance suite.
* `volatile_load` / `volatile_store` / `volatile_load_acquire` /
  `volatile_store_release` intrinsics are mounted from
  `core.intrinsics.lowlevel.mmio` and tested at that layer's
  conformance surface.

## 1. Cross-stdlib usage

The MMIO layer is consumed by every bare-metal driver in the embedded
support surface (none currently in stdlib — driver crates live
out-of-tree) and by the runtime atomics dispatch when the
`BarrierKind` variant decision needs to flow into a `barrier(...)`
intrinsic call. The `MemoryFlags` bitfield is used by the
`MemoryRegion` linker-symbol abstraction for permission tracking on
embedded targets.

## 2. Crate-side hardcodes

| Site | What it pins | Status |
|---|---|---|
| `crates/verum_vbc/src/intrinsics/registry.rs` | `volatile_load` / `volatile_store` / barrier intrinsic registrations | OK |
| `crates/verum_codegen/src/llvm/mmio.rs` | Volatile pointer → LLVM `volatile` attribute on load/store | OK |
| `crates/verum_codegen/src/llvm/atomics.rs` | `BarrierKind` → LLVM fence ordering parameter | OK |

## 3. Language-implementation gaps surfaced by this suite

### 3.1 Phantom-type access modes (no defect — pinned as design choice)

`ReadOnly`, `WriteOnly`, `ReadWrite`, `ReadWriteOnce`,
`WriteOneToClear`, `WriteOneToSet` are unit types (`()` variants)
used as compile-time access-mode tags on `MmioRegister<T, MODE>`.
The current test suite doesn't exercise them at the type level
because the conformance suite runs only at the value level; the
type-level invariants are pinned by the type-checker's monomorphisation
pass directly. No defect; tracked as a deliberate scope boundary.

### 3.2 `*volatile mut T` pointer type (deferred runtime fixture)

The volatile-pointer type is correctly type-checked end-to-end, but
runtime fixture for actual MMIO load/store would require a mocked
memory page that's not yet wired into the test infrastructure.
Tracked as a deferred action item.

## 4. Action items landed in this branch

1. **`unit_test.vr`** — 19 @tests covering BarrierKind, MemoryFlags,
   MemoryRegion + the two thin helper aliases (`compiler_barrier`,
   `dmb`).
2. **`regression_test.vr`** — 5 @tests pinning re-export
   resolution, tuple-newtype constant width preservation, and
   method dispatch on records.

## 5. Action items deferred

1. **Runtime MmioRegister load/store coverage** — requires a memory
   fixture. Estimate: 2 days once the fixture infrastructure lands.
2. **VerifiedRegister ghost-state pinning** — requires SMT
   verification driver integration; tracked under the
   verum_verification crate's broader proof-coverage push.
3. **`RegisterBlock` protocol coverage** — depends on the runtime
   MmioRegister surface landing first.
