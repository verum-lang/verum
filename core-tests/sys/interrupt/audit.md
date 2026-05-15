# `core.sys.interrupt` — implementation audit

## Status: **regression-only** (kernel-mode surface; user-space test coverage gates only the type-shape pins)

* This module exposes interrupt-handler / critical-section / context-
  switching primitives. Every meaningful runtime behaviour requires
  ring-0 / kernel-mode execution (or the embedded baremetal target),
  which is **out of scope** for the in-process `verum test` harness
  running under macOS/Linux user-space.
* What IS tested here: type-shape pins on the public ADTs
  (`CriticalSection`, `InterruptCell<T>`) so that future regressions
  in compile-time generic-substitution / record-construction surface
  on this module immediately.
* The full interrupt-control surface is exercised by the VCS specs
  under `vcs/specs/L0-critical/` and the embedded runtime
  integration suite under `core-tests/sys/embedded/` (when that
  lands).

## 1. Cross-stdlib usage

`core.sys.interrupt` is consumed by every `@interrupt(vector = N)`
attributed handler in the embedded runtime + the V-LLSI signal
handler family (which uses `CriticalSection` to atomically swap the
signal mask).

## 2. Action items landed in this branch

1. `unit_test.vr` — 3 `@test`s pinning the user-space surface
   (shape of `is_active()` + `InterruptCell.new`).
2. `regression_test.vr` — 2 `@test`s pinning the generic-arg
   substitution path through `InterruptCell<T>.new(value)`.

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Critical-section lock acquisition + nesting | Kernel-mode; tested in VCS specs. |
| 2 | `with_interrupts_disabled<R, F>` closure dispatch | Needs interrupt-fixture infrastructure. |
| 3 | `context_switch` / `CpuContext` round-trip | Embedded-only. |
