# `core.sys.init` — implementation audit

## Status: **partial** (type-level surface complete; bootstrap-time entry points deferred)

* `InitError` 6-variant algebra is pinned by `unit_test.vr` +
  `property_test.vr` (reflexivity / symmetry / cross-variant
  distinctness across the full domain).
* `.message()` is pinned per-variant with a canonical text.
* `is_initialized()` shape pin via the Bool round-trip in
  `unit_test.vr` and `regression_test.vr` §B.
* **`verum_init` / `verum_shutdown` are NOT pinned at this layer** —
  these are bootstrap-time entry points; running them under the test
  harness would re-trigger the bootstrap sequence the harness already
  drove. The bootstrap is exercised indirectly by every other test
  (every test only runs because verum_init succeeded already).
* `panic_impl` / `set_panic_handler` similarly NOT exercised — they
  terminate the process when triggered, which is incompatible with
  the test harness running multiple `@test` functions in the same
  process.

## 1. Cross-stdlib usage

`core.sys.init` is the V-LLSI bootstrap kernel entry point. Consumers:

| Consumer | Touches | Notes |
|---|---|---|
| Platform `_start` (Linux x86_64 / aarch64) | `verum_init` | First user-space callable post-kernel. |
| `main.c` (libSystem-routed macOS) | `verum_init` | dyld initializer. |
| `mainCRTStartup` (Windows) | `verum_init` | TLS callbacks. |
| `Reset_Handler` (embedded) | `verum_init` | bare-metal reset path. |

## 2. Crate-side hardcodes

| Site | What it pins | Status |
|---|---|---|
| `crates/verum_codegen/src/llvm/entry_point.rs` | platform-canonical entry (_start, main, etc.) | OK |

## 3. Action items landed in this branch

1. `unit_test.vr` — 11 `@test`s covering InitError construction,
   message contents, Eq for unit + payload variants, and the
   `is_initialized()` shape pin.
2. `property_test.vr` — 5 algebraic-law `@test`s exhausting the
   6-variant Eq domain.
3. `integration_test.vr` — 2 `@test`s composing InitError with
   `Result<T, InitError>` and `List<InitError>`.
4. `regression_test.vr` — 3 `@test`s pinning Eq + shape.

## 4. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | `verum_init` / `verum_shutdown` happy/error paths | Bootstrap-time; out of scope for in-process tests. |
| 2 | `panic_impl` / `set_panic_handler` | Termination-time; out of scope for in-process tests. |
| 3 | `init_thread` / `cleanup_thread` per-thread lifecycle | Belongs in `core-tests/async/` where the right tier is wired. |
