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

## 2.1 Action items landed in 2026-05-27 conformance refresh

1. `property_test.vr` — 8 `@test`s, including the now-active
   `CriticalSection.is_active()` idempotent-reads + user-space-false
   contract pins (Section 0), plus 6 `InterruptCell<T>.new` sweeps over
   the full primitive parameter-type domain (Int / Text / Bool, plus
   negative-Int and Int.min / Int.max boundary payloads) and a
   no-side-effects assertion that the constructor is `@const` over the
   captured local.
2. `integration_test.vr` — 7 `@test`s, including `CriticalSection.is_active`
   funnelled through `Maybe<Bool>` and a two-read tuple-consistency
   assertion (Section 2), plus 5 `InterruptCell<T>` × `List` × custom-
   record composition tests.
3. Closed defect class **§3.4** — replaced the `@intrinsic` externs in
   `core/intrinsics/lowlevel/kernel.vr` (`disable_interrupts`,
   `enable_interrupts`, `restore_interrupts`, `interrupts_enabled`,
   `save_and_disable_interrupts`) with safe-default Verum bodies
   (returns `true` / `0` / no-op for the host target). Kernel /
   embedded targets override via `@cfg(no_runtime)` or a platform
   module. Verified GREEN under `--interp` after binary co-rebuild
   (`cargo build --bin verum --release`); the previously-failing
   `test_critical_section_is_active_returns_bool` now passes.

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Critical-section lock acquisition + nesting | Kernel-mode; tested in VCS specs. |
| 2 | `with_interrupts_disabled<R, F>` closure dispatch | Needs interrupt-fixture infrastructure. |
| 3 | `context_switch` / `CpuContext` round-trip | Embedded-only. |
| 4 | **Kernel-intrinsic registry gap** | **CLOSED 2026-05-27**: replaced `@intrinsic` externs in `kernel.vr` with safe-default Verum bodies for host target. `interrupts_enabled` → `true`; `disable_interrupts` / `save_and_disable_interrupts` → return `0`; `enable_interrupts` / `restore_interrupts(_)` → no-op. Kernel/embedded path will need `@cfg(no_runtime)` override module — tracked separately under task #embedded-intrinsics. |
