# `core.mem.diagnostics` — audit findings

> Module under test: `core/mem/diagnostics.vr` (471 LOC; 1 record
> `MemHeaderView` with 8 fields + Display/Debug + `from_header`
> constructor + a few accessor methods, 1 record `CallFrame` with 5
> fields + Display/Debug, 2 free functions `live_allocations` and
> `live_allocation_count`, 1 free function `current_call_stack`).
>
> Test surfaces (this branch):
> `unit_test.vr` (~120 LOC), `property_test.vr` (~95 LOC),
> `integration_test.vr` (~120 LOC), `regression_test.vr` (~95 LOC).
>
> The module's mutation surface is EMPTY by design — every type and
> function is read-only.  Tests pin the snapshot-record shapes; the
> producer-side (writing MemHeaderView snapshots) is owned by the
> CBGR allocator and tested separately.

## 1. Cross-stdlib usage

| Consumer | Use |
|---|---|
| `core/mem/header.vr` | `MemHeaderView.from_header(header)` captures a snapshot. |
| Future panic handler | Calls `current_call_stack(skip)` + `live_allocations()` for post-mortem dumps. |
| Future runtime monitor | Polls `live_allocation_count()` for memory pressure / leak detection. |

## 2. Crate-side hardcodes

Drift surfaces:

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `MemHeaderView` 8-field record layout | Snapshot shape matches the underlying `AllocationHeader` 1:1 | Adding a field to AllocationHeader without updating MemHeaderView = silent data loss in observers. Pinned by `regression_test §A`. |
| `CallFrame` 5-field record layout | Stack-frame summary shape | Pinned by `regression_test §D`. |
| `live_allocations` returns `List<MemHeaderView>` (not iterator) | Caller obligation: full snapshot at observation time | An iterator would be more efficient but allows mutation-during-iteration races; the list-snapshot is the safer default. |

## 3. Language-implementation gaps

### 3.1 `live_allocations` is currently empty-stub

Pre-sidecar, `live_allocations()` returns `List.new()` and
`live_allocation_count()` returns `0`. The producer-side global
allocation index is future work. Tests pin the contract (list and
count consistent) and the no-panic invariant.

### 3.2 `current_call_stack` is currently empty-stub

Same situation. Tests verify it accepts any skip value without
panic.

### 3.3 No mutation surface by design

Every type in this module is a snapshot — no setters, no mutating
methods. This is the load-bearing invariant for safe observer-side
usage (panic handler must not mutate runtime state).

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| 1 | Missing test coverage for `core/mem/diagnostics.vr` | `core-tests/mem/diagnostics/{unit,property,integration,regression}_test.vr` | New 4-file suite; ~430 LOC total. |
| 2 | Missing `audit.md` for `core-tests/mem/diagnostics/` | This file. |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Wire the producer-side global allocation index so `live_allocations` returns real data. | ~4 hours | open |
| §B | Wire `current_call_stack` to the VBC debug-info section so it returns actual frames. | ~3 hours | open |
| §C | Cross-tier divergence sweep on `--aot` + `--interp`. | 1 hour wall-clock | open |
| §D | Test `MemHeaderView.from_header(&AllocationHeader)` — requires a live header pointer (deferred to `core-tests/mem/allocator/`). | Blocked on `mem/allocator/` | open |
