# `core.sys.mod` — implementation audit

## Status: **complete** (under `--interp`; umbrella surface)

* This module IS the umbrella `core.sys` namespace — it re-exports the
  V-LLSI public surface across `common`, `bitfield`, `mmio`,
  `interrupt`, `io_engine`, `init`, and the platform-conditional
  submodules.
* The conformance suite here verifies the umbrella RE-EXPORT contract:
  every type / constant / function reachable through
  `mount core.sys.X;` (the canonical user-facing import path) is
  byte-identical to the same name reached via the direct submodule
  path (`mount core.sys.<submod>.X;`).
* All other umbrella-touching surface is covered by the submodule
  conformance suites; this audit pins only the re-export shape.

## 1. Cross-stdlib usage

`core.sys` is the entry point for every consumer of V-LLSI
primitives:

| Caller | Path |
|---|---|
| `core.io.*` | OSError funnel, FileDesc, page-aligned buffers. |
| `core.net.*` | Fd, IOEngine, TimeSpec. |
| `core.mem.*` | PAGE_SIZE, page_align_up/down, MemProt. |
| `core.async.*` | TimeSpec, IoEngine. |
| `core.intrinsics.*` | Constants flow back through `runtime/os.vr`. |
| User code | The whole umbrella — direct submodule mounts are not the canonical path. |

## 2. Action items landed in this branch

1. `unit_test.vr` — 17 `@test`s pinning umbrella reachability across
   constants (PAGE_SIZE, MAX_CONTEXT_SLOTS, CONTEXT_STACK_DEPTH,
   USIZE_BITS) + common types (OSError, SysContextError, MemProt,
   MemoryOrdering, MapFlags) + io_engine types (Fd, TimeSpec) +
   mmio types (BarrierKind, MemoryFlags, MemoryRegion) + init types
   (InitError).
2. `property_test.vr` — 10 algebraic laws verifying:
   - Path-equivalence: umbrella constant value == direct-submodule
     value (`PAGE_SIZE` ↔ `common.PAGE_SIZE`, `MAX_CONTEXT_SLOTS` ↔
     `common.MAX_CONTEXT_SLOTS`, `CONTEXT_STACK_DEPTH` ↔
     `common.CONTEXT_STACK_DEPTH`, `USIZE_BITS` ↔ `bitfield.USIZE_BITS`).
   - Umbrella PAGE_SIZE invariants (positive + power-of-two + ≥ 4096).
   - MAX_CONTEXT_SLOTS positive + power-of-two.
   - USIZE_BITS byte-multiple + ≥ 32.
3. `integration_test.vr` — 8 cross-stdlib scenarios touching the
   umbrella path: OSError × List<OSError> sum-fold; Fd × Maybe<Fd>
   open-funnel; TimeSpec × Result<TimeSpec, OSError>; BarrierKind
   strength dispatch table; MemoryFlags OR composition (text segment);
   InitError × Maybe lift; PAGE_SIZE in alignment arithmetic;
   USIZE_BITS in bit-shift expressions.
4. `regression_test.vr` — 4 `@test`s pinning type-identity preservation
   across the umbrella boundary (OSError field round-trip; Fd newtype
   round-trip; PAGE_SIZE platform invariant; USIZE_BITS drift pin
   against the canonical `USize.bits` intrinsic).

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Platform-conditional re-export reachability | `linux.*` / `darwin.*` / `windows.*` umbrella mounts are `@cfg(target_os=…)`-gated; the host-build path only exposes ONE platform. Lives in per-platform conformance suites. |
| 2 | `@cfg(runtime = "embedded")` umbrella re-exports for `embedded.*` + `no_runtime.*` | Same as #1; tracked under `vcs/specs/L0-critical/`. |
| 3 | Cross-module bare-import shadowing | The deliberate omission of bare `read` / `write` / `close` aliases in `core/sys/{linux,darwin}/mod.vr` re-exports — pinned at the source level (`# SOUNDNESS:` comment block); regression-pin in `core-tests/sys/io_engine/regression_test.vr` already locks this in. |
