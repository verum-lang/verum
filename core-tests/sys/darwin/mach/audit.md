# `core.sys.darwin.mach` — implementation audit

## Status: **partial** (constant + newtype surface complete; live Mach FFI deferred)

* This module exposes the Mach kernel primitive types
  (`KernReturn`, `MachPort`, `VmAddress`, `VmSize`, `VmProt`,
  `SemaphoreT`, `MachMsgHeader`) plus the canonical kernel-return,
  VM-protection, VM-flags, and sync-policy constants.
* The Mach FFI bindings (`vm_allocate` / `vm_deallocate` / `vm_protect`
  / `semaphore_create` / `semaphore_signal` / `mach_absolute_time`)
  require live Mach ports and the privileged task port — out-of-scope
  for in-process conformance.

## 1. Cross-stdlib usage

`core.sys.darwin.mach` underlies macOS-specific memory + synchronisation
primitives:

| Caller | Use |
|---|---|
| `core.sys.darwin.libsystem` | mmap-equivalents via `vm_allocate`. |
| `core.sys.darwin.thread` | Semaphore-based wakeups, `__ulock_*`. |
| `core.sys.darwin.time` | `mach_absolute_time` + `mach_timebase_info`. |

## 2. Action items landed in this branch

1. `unit_test.vr` — 17 `@test`s pinning KERN_* return codes (0..=14
   sample), VM_PROT_* canonical values (0, 1, 2, 4, 3, 7),
   VM_FLAGS_FIXED/ANYWHERE/PURGABLE, SYNC_POLICY distinctness,
   KernReturn / VmProt newtype round-trip.
2. `property_test.vr` — 7 algebraic laws: KERN_* consecutive 0..=8
   sequence + KERN_ABORTED at 14; KERN_SUCCESS-is-falsy contract;
   VM_PROT_* power-of-two + pairwise-disjoint + DEFAULT = READ|WRITE +
   ALL = READ|WRITE|EXECUTE; VM_FLAGS distinctness.
3. `regression_test.vr` — 3 `@test`s pinning the Int32 width on
   KernReturn (defect class from T0.4.2 Phase 3 — was (Int) i64
   causing AOT register garbage), KERN_SUCCESS=0 invariant,
   VM_PROT_ALL canonical value.

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Live Mach FFI exercise | `vm_allocate` / `vm_deallocate` / `vm_protect` / `semaphore_*` require live Mach ports — VCS specs domain. |
| 2 | `MachMsgHeader` round-trip | Mach IPC layout — out of scope for user-space conformance. |
| 3 | Full kern_return_t catalogue | The 25+ constants beyond KERN_OPERATION_TIMED_OUT exist in the source but aren't conformance-pinned. |
