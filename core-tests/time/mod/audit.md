# `core.time` (module manifest + `Time` namespace) — audit findings

> Module under test: `core/time/mod.vr` (~180 LOC; re-export manifest
> for the 8 submodules + the `Time` unit-type namespace with
> `now` / `monotonic` / `sleep` / `sleep_ms` / `sleep_secs`).
>
> Test surfaces (this branch): `unit_test.vr`, `property_test.vr`,
> `integration_test.vr`, `regression_test.vr`. This folder closes the
> mirror-contract gap — `mod.vr` previously had NO test folder.

## 1. Cross-stdlib usage

`Time` is the blocking-clock façade. Consumers:

| Consumer | Use |
|---|---|
| `core.time.interval.Interval` | `Time.monotonic()` for deadlines, `Time.sleep()` inside `tick()` |
| `core.time.interval.AsyncInterval` | `Time.monotonic()` in `poll_next` |
| retry/backoff helpers (`core/base/retry.vr`) | blocking sleeps between attempts |
| ad-hoc benchmarks in `vcs/` | `Time.monotonic()` deltas |

The async stack does NOT go through `Time.sleep` (it uses
`core/async/timer.vr` futures) — correct layering.

## 2. Crate-side hardcodes / drift surfaces

* `crates/verum_vbc/src/codegen/mod.rs::register_stdlib_intrinsics` —
  free time functions (`monotonic_nanos`, `realtime_nanos`,
  `sleep_ms`, `sleep_us`, `sleep`) are name-keyed intrinsic aliases.
  **Drift risk**: the `("sleep", 1, …)` row matches ANY 1-arg function
  named `sleep`, including `core/async/timer.vr::sleep(Duration) ->
  Sleep` (which must return a future, not block). Deferred item D2.
* 2026-07-09: the `Duration.*` / `Instant.*` rows were REMOVED from the
  same table (§G closure — see `duration/audit.md §G`). `Time.now()`
  itself was a §G casualty: it builds a Duration via
  `Duration.from_nanos`, which was intercepted (raw Int) while
  accessors on the result ran Verum bodies (heap record) — pinned by
  `regression_b_now_round_trips_through_accessors`.
* Interpreter Int-receiver fallbacks
  (`method_dispatch.rs` "as_secs"/"elapsed"/"duration_since" on raw
  Int receivers) still exist for Stopwatch/PerfCounter raw flows.
  They can hijack a user's `Int.elapsed()`; deferred item D3.

## 3. Language-implementation gaps pinned here

* **§A `Time.sleep(negative)` infinite hang** — `as UInt64` on a
  negative nanosecond count. Language-level lesson: signed→unsigned
  `as` casts are silent bit reinterpretations; every stdlib boundary
  crossing into an unsigned syscall ABI needs an explicit clamp or a
  checked conversion. Fixed with a clamp; pinned by
  `regression_a_negative_sleep_does_not_hang`.
* **§B dual-representation intercepts** (§G family) — see
  `duration/audit.md §G`; the manifest's own `Time.now()` was
  affected. Fundamental fix landed 2026-07-09: representation is
  uniformly "heap record"; the Verum bodies are the single
  implementation surface on both tiers.

## Action items landed in this branch

* Clamp added to `Time.sleep` (non-positive spans return immediately).
* Signed-period clamp in `Interval.new` / `Interval.immediate` /
  `AsyncInterval.new`.
* Full mirror test folder for `mod.vr` (this folder).
* §G closure in `verum_vbc` (alias-table rows removed; see
  `duration/audit.md` for the full defect narrative).

## Action items deferred

* **D1 (small)**: `@cfg`-gated `sys_sleep` mounts — no Windows CI leg
  exercises the `windows` arm of `Time.sleep`.
* **D2 (medium)**: name-keyed `("sleep", 1, …)` intrinsic alias can
  shadow `core/async/timer.vr::sleep` — replace name-keyed alias
  registration with module-qualified keys.
* **D3 (medium)**: remove interpreter Int-receiver name-keyed
  fallbacks ("as_secs", "elapsed", …) once Stopwatch/PerfCounter move
  to typed records; they mask method-resolution failures.
* **D4 (design)**: transparent-newtype representation optimisation —
  a typed, whole-path contract (`@repr(transparent)`-class) to recover
  the zero-cost accessor inlining that the removed §G alias table
  attempted, without dual representation. Needs construction, field
  access, dispatch, and pattern-match coverage in one design.
