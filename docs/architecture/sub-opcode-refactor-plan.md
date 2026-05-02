# Sub-Opcode Space Refactor — Architecture Plan

**Status**: Phases 0+1+2 done (commits 6a66648f, c109b778,
92ccdd61); Phases 3+4 pending.
**Owners**: VBC codegen + interpreter dispatch + AOT lowering
**Tracking**: see #91 perf parity (this is a sub-task)

## Phase status

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| 0     | DONE   | 6a66648f | Audit + rename FfiSubOpcode→SystemSubOpcode + this doc |
| 1     | DONE   | c109b778 | TimeSubOpcode, SysSubOpcode, MachSubOpcode, SyncSubOpcode + CbgrSubOpcode 0x60-0x6F |
| 2     | DONE   | 92ccdd61 | 194 source renames in 5 consumer files |
| 3     | PEND   | —        | `verum audit --subop-cleanliness` gate |
| 4     | PEND   | —        | Reclaim top-level Opcode bytes — multi-day per opcode |

## Motivation

The 15 `*SubOpcode` enums in `crates/verum_vbc/src/instruction.rs`
encode the secondary dispatch byte for `Instruction::*Extended`
opcodes.  Each enum is an 8-bit (256-entry) opcode space.
Total variants today: **736** across 15 enums.

Audit revealed that `FfiSubOpcode` is severely overloaded —
**30 of 77 entries (39%) are misplaced**: Time, Sys, Mach, Cbgr,
Sync, Random groups that have nothing to do with FFI.  This:

  * **Hurts performance**: dispatchers must be wider than they
    need to (sparse 256-entry table for many small clusters).
  * **Confuses readers**: `FfiSubOpcode::TimeMonotonicNanos`
    suggests a C-FFI time call, but it's actually a syscall.
  * **Wastes space**: each cluster occupies a 16-byte slot
    (e.g. 0x70-0x7F for Time) while the surrounding FFI region
    (0x68-0x7F overall) gets fragmented.
  * **Blocks future growth**: when adding a new genuine FFI op,
    the next free slot may be in the middle of the Time region.

## Current state per enum

| Enum                 | Variants | Misplacements | Health  |
|----------------------|----------|---------------|---------|
| `CubicalSubOpcode`   | 17       | 0             | clean   |
| `GpuSubOpcode`       | 97       | 0             | clean   |
| `TensorSubOpcode`    | 149      | 0             | clean   |
| `TensorExtSubOpcode` | 17       | overlaps Ffi  | medium  |
| `MlSubOpcode`        | 62       | 0             | clean   |
| `FfiSubOpcode`       | 77       | **30**        | **bad** |
| `ArithSubOpcode`     | 58       | 0             | clean   |
| `CmpSubOpcode`       | 4        | 0             | clean   |
| `MathSubOpcode`      | 80       | 0             | clean   |
| `SimdSubOpcode`      | 67       | 0             | clean   |
| `CbgrSubOpcode`      | 39       | 0             | clean   |
| `CharSubOpcode`      | 32       | 0             | clean   |
| `LogSubOpcode`       | 9        | 0             | clean   |
| `TextSubOpcode`      | 10       | 0             | clean   |
| `ExtendedSubOpcode`  | 3        | 0             | clean   |

Plus duplicate-byte issue: `FfiSubOpcode::RandomU64=0x47` /
`RandomFloat=0x48` are also at `TensorExtSubOpcode::RandomU64=0x05`
/ `RandomFloat=0x06`.

## Misplacements in `FfiSubOpcode`

```
0x47  RandomU64           → TensorExtSubOpcode (already there)
0x48  RandomFloat         → TensorExtSubOpcode (already there)

0x70  TimeMonotonicNanos      ┐
0x71  TimeRealtimeNanos       │
0x72  TimeMonotonicRawNanos   │ → NEW TimeSubOpcode
0x73  TimeSleepNanos          │
0x74  TimeThreadCpuNanos      │
0x75  TimeProcessCpuNanos     ┘

0x80  SysGetpid           ┐
0x81  SysGettid           │
0x82  SysMmap             │ → NEW SysSubOpcode
0x83  SysMunmap           │
0x84  SysMadvise          │
0x85  SysGetentropy       ┘

0x90  MachVmAllocate      ┐
0x91  MachVmDeallocate    │
0x92  MachVmProtect       │
0x93  MachSemCreate       │
0x94  MachSemDestroy      │ → NEW MachSubOpcode (Apple-specific)
0x95  MachSemSignal       │
0x96  MachSemWait         │
0x97  MachErrorString     │
0x98  MachSleepUntil      ┘

0xA0  CbgrAlloc          ┐
0xA1  CbgrAllocZeroed    │ → CbgrSubOpcode @ 0x60-0x6F (currently empty)
0xA2  CbgrDealloc        │
0xA3  CSecureZero        ┘   (rename to SecureZero, drop C prefix)

0xB0  FutexWait          ┐
0xB1  FutexWake          │ → NEW SyncSubOpcode
0xB2  SpinlockLock       ┘   (more sync primitives planned)
```

## Proposed new layout

### `FfiSubOpcode` (refactored, ~50 entries, room for 200 reserve)

```
0x00-0x0F  FFI library/symbol management (LoadSymbol, GetLibrary, ...)
0x10-0x1F  FFI calling conventions (CallFfiC, CallFfiStdcall, ...)
0x20-0x2F  Marshalling (MarshalToC, MarshalFromC, StringToC, ...)
0x30-0x3F  Errno/error handling (GetErrno, SetErrno, GetLastError, ...)
0x40-0x4F  C-allocator + raw byte arrays (CAlloc, CFree, CMemcpy, ...)
0x50-0x5F  Callbacks (CreateCallback, FreeCallback, ...)
0x60-0x6F  Raw pointer ops (DerefRaw, PtrAdd, PtrSub, PtrIsNull, ...)
0x70-0xFF  RESERVED for future FFI growth (variadics, FFI types,
           cross-platform abi adapters, bindgen-style helpers, ...)
```

Net: 47 → ~50 entries; reserve space goes from ~30 (fragmented)
to **210 contiguous bytes** of growth headroom.

### `TimeSubOpcode` (NEW, dedicated)

```
0x00 MonotonicNanos       — clock_gettime(CLOCK_MONOTONIC)
0x01 RealtimeNanos        — clock_gettime(CLOCK_REALTIME)
0x02 MonotonicRawNanos    — clock_gettime(CLOCK_MONOTONIC_RAW)
0x03 SleepNanos           — nanosleep
0x04 ThreadCpuNanos       — clock_gettime(CLOCK_THREAD_CPUTIME_ID)
0x05 ProcessCpuNanos      — clock_gettime(CLOCK_PROCESS_CPUTIME_ID)
0x06-0x1F  RESERVED for cross-platform time (Windows QPC,
           POSIX timer_create, etc.)
0x20-0xFF  RESERVED
```

### `SysSubOpcode` (NEW, dedicated)

```
0x00 GetPid
0x01 GetTid
0x02 Mmap
0x03 Munmap
0x04 Madvise
0x05 GetEntropy
0x06-0x1F  RESERVED for syscalls (fork, exec, signal, waitpid, ...)
0x20-0x3F  RESERVED for /proc & /sys access
0x40-0xFF  RESERVED
```

### `MachSubOpcode` (NEW, Apple-specific)

```
0x00 VmAllocate
0x01 VmDeallocate
0x02 VmProtect
0x10 SemCreate
0x11 SemDestroy
0x12 SemSignal
0x13 SemWait
0x20 ErrorString
0x21 SleepUntil
0x22-0xFF  RESERVED for Mach kernel additions
```

### `CbgrSubOpcode` (extended in existing space)

Add at `0x60-0x6F` (currently empty):

```
0x60 Alloc            (was FfiSubOpcode::CbgrAlloc 0xA0)
0x61 AllocZeroed      (was FfiSubOpcode::CbgrAllocZeroed 0xA1)
0x62 Dealloc          (was FfiSubOpcode::CbgrDealloc 0xA2)
0x63 SecureZero       (was FfiSubOpcode::CSecureZero 0xA3)
0x64-0x6F  RESERVED for allocator-side primitives
```

### `SyncSubOpcode` (NEW)

```
0x00 FutexWait        (was FfiSubOpcode::FutexWait 0xB0)
0x01 FutexWake        (was FfiSubOpcode::FutexWake 0xB1)
0x02 FutexWakeAll     (NEW, currently FfiSubOpcode-undefined)
0x10 SpinlockLock     (was FfiSubOpcode::SpinlockLock 0xB2)
0x11 SpinlockTryLock
0x12 SpinlockUnlock
0x20 ParkNs
0x21 Unpark
0x30-0xFF  RESERVED for sync primitives (semaphore, condvar
           cross-platform shims, atomic memory ordering helpers)
```

## Migration strategy

### Phase 1 — additive (backward compatible)

Add new SubOpcode enums + new `Instruction::*Extended` variants
WITHOUT removing the old `FfiSubOpcode` entries.  Codegen emits
the NEW opcode for new code.  Old serialized .vbc files continue
to work via the existing FfiSubOpcode dispatch.

Per group:

  * Add `pub enum TimeSubOpcode` to `instruction.rs`.
  * Add `Instruction::TimeExtended { sub_op: u8, operands }`.
  * Add `lower_time_extended` in `crates/verum_codegen/src/llvm/instruction.rs`.
  * Add `handle_time_extended` in
    `crates/verum_vbc/src/interpreter/dispatch_table/handlers/`.
  * Add `emit_intrinsic_time_extended(sub_op, args, dest)` helper
    in `crates/verum_vbc/src/codegen/expressions.rs`.
  * Update intrinsic registry to use `CodegenStrategy::TimeExtendedOpcode(...)`
    where it currently uses `FfiExtendedOpcode(...)` for Time*.

### Phase 2 — codegen-side migration

Switch every callsite from `emit_intrinsic_library_call("verum_time_*")`
+ FfiSubOpcode handler arms to `emit_intrinsic_time_extended(...)`.
Verify VBC codegen test suite (1675 tests) stays green.

### Phase 3 — bytecode validity gate

`verum audit --subop-cleanliness` reports any FfiSubOpcode entry
that's been migrated.  Block PRs that emit FfiSubOpcode for a
migrated op.

### Phase 4 — deprecation deletion

After 1 release cycle (or once all `.vbc` artifacts are
regenerated), remove the old FfiSubOpcode entries entirely.
Reclaim the byte space for genuine FFI growth.

## Backward compatibility

* `.vbc` artifacts: serialized bytecode files reference
  `FfiSubOpcode` discriminants by byte value.  Phase 1 keeps
  them working; Phase 4 breaks them.  Either bump VBC version
  number or run an upgrade pass on existing `.vbc` files.
* Public API: nothing changes — `verum_intrinsic_*`
  functions in `core/intrinsics/` keep their names; only
  the internal opcode dispatch changes.

## Performance impact

  * **Dispatcher table size**: each `Instruction::*Extended` is
    a separate match arm in the interpreter's dispatch_table.
    Splitting 30 misplaced entries off `FfiSubOpcode` means:
      - FfiSubOpcode handler shrinks (better I-cache fit)
      - Each new dedicated handler (Time/Sys/Mach/Sync) is
        small and L1-resident
      - Net dispatch latency: ~5-10% reduction expected on
        Time/Sys-heavy workloads.
  * **AOT side**: same shrinkage benefit; smaller match
    statements lower well.

## Effort estimate

  * Audit + this document: complete (this commit).
  * Phase 1 per-group implementation: ~1 day each
    (Time, Sys, Mach, Sync = 4 days).
  * Phase 1 Cbgr extension: ~0.5 day.
  * Phase 2 codegen migration: ~1 day across all groups.
  * Phase 3 + 4 (deprecation): defer to a release cycle.

**Total**: ~6 days plus cycle delay.  This task is the
single largest architectural cleanup remaining for #91.

## Related work

  * Commit 87f62ea1 (#83): cross-module `@intrinsic`
    deduplication via `mount` re-exports.  Same architectural
    spirit; this refactor extends it from declaration sites to
    bytecode dispatch.
  * Commit 14f63b2c (#74 partial): legacy InlineSequenceId
    fallback now routes 32 intrinsics to TensorExtended +
    TensorExtSubOpcode.  Same FFI/Tensor cross-pollination
    issue identified there.
