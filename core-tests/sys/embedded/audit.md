# `core.sys.embedded` — implementation audit

## Status: **partial** (data-shape + bump-allocator arithmetic complete; volatile MMIO I/O deferred)

* This module ships the minimal runtime for embedded / bare-metal
  Verum builds — activated at the umbrella level by
  `@cfg(runtime = "embedded")` (see `core/sys/mod.vr`). The types
  themselves are NOT @cfg-gated, so they are reachable from the
  default user-space build via direct mount (`mount core.sys.embedded.X`)
  — this conformance suite exercises that surface.
* Public API:
  - `StackAllocator` — bump allocator over a fixed buffer.
  - `RingBuffer` — fixed-size circular buffer for MMIO/UART I/O.
  - `PanicAction` — 3-variant (Halt | Reset | Custom(fn() -> ())).
  - `set_panic_action(action: PanicAction)`, `embedded_panic()` —
    global panic-handler registration / dispatch.

## 1. Cross-stdlib usage

`core.sys.embedded` is the bottom of the embedded runtime stack;
under `@cfg(runtime = "embedded")` it replaces the heap, the async
executor, and (via `core.sys.no_runtime`) the channel/mutex
primitives.

| Caller (embedded-only) | Use |
|---|---|
| `core.sys.no_runtime` | Async-to-sync stubs (block_on, spawn_sync). |
| `core.sys.mmio` | Volatile load/store primitives via `volatile_load`/`volatile_store`. |
| User firmware code | Direct stack allocation + UART ring buffer. |

## 2. Action items landed in this branch

1. `unit_test.vr` — 19 `@test`s pinning StackAllocator construction
   (capacity / used / remaining accessors at zero/full states),
   bump arithmetic (alignment rounding, sequential allocs sum),
   reset semantics, OOM sentinel (returns 0), and RingBuffer
   construction (empty/full/len/reset invariants over both positive
   and zero capacities) + PanicAction variant construction.
2. `property_test.vr` — 13 `@test`s sweeping algebraic laws:
   `used() + remaining() == capacity()` invariant; remaining
   monotone-decreasing under alloc; reset restores remaining to
   capacity; alignment-relative-to-buffer-base contract; OOM iff
   overflow with no-advance commit; sequential-allocs-sum identity;
   RingBuffer empty/full/len/capacity bounds; 3-variant PanicAction
   exhaustive dispatch.
3. `integration_test.vr` — 8 cross-stdlib scenarios composing
   StackAllocator with Maybe<Int> OOM funnel, a struct-table
   bootloader pattern (table + 4 descriptors), two-independent-
   allocators coexistence, RingBuffer state-machine invariants,
   PanicAction classification + Maybe<PanicAction> lift, and a
   List<Int> pointer-collection sweep with ascending-order guard.
4. `regression_test.vr` — 4 `@test`s pinning known defect classes:
   bump-allocator overflow ordering (OOM does NOT advance offset);
   alignment-relative-to-buffer-base contract; RingBuffer.reset
   yields empty (not full); zero-capacity RingBuffer construction.

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | RingBuffer.push / pop semantic round-trip | `volatile_load` / `volatile_store` at synthetic pointer (0x1000) would UB on a host. Lives in `vcs/specs/L0-critical/embedded/` against a controlled MMIO fixture. |
| 2 | `set_panic_action(action)` global-state mutation | Touches `static mut PANIC_ACTION` — needs `@cfg(runtime = "embedded")` build target to exercise (host build elides the mutable static). |
| 3 | `embedded_panic()` dispatch | Same as #2; the `loop {}` halt path is not testable from a normal `@test`. |
| 4 | `Custom(handler)` invocation | Requires `@cfg(runtime = "embedded")` build to dispatch through `embedded_panic()`. Function-value extraction works (Section 3 of integration_test pins the variant payload classifier). |
