# `core.sys.context_ops` — implementation audit

## Status: **partial** (raw-intrinsic mount migration landed; runtime DI surface pinned at the wrapper level)

* Every public function on the wrapper surface (`tls_get` / `tls_set` /
  `context_provide` / `context_get` / `context_end` / `defer_register` /
  `defer_execute` / `defer_depth` / `defer_run_to`) is covered by
  `unit_test.vr` round-trip tests where the V-LLSI contract is
  expressible at the type-level.
* `TLS_SLOT_COUNT` constant is pinned at `256` end-to-end, and the
  power-of-two / strictly-positive structural invariants are pinned in
  `property_test.vr`.
* `integration_test.vr` exercises three real user-side patterns: a
  TLS-backed per-thread counter, a Maybe-shaped wrapping of a TLS read,
  and a multi-type_id context-provide chain over a List+fold pattern.
* The **`defer_register`/`defer_execute`/`defer_run_to` family** is
  deferred at the property/integration layer because the canonical user
  pattern requires a function pointer (`fn(Int) -> Int`) — testing the
  full lifecycle would need to compose with the closure-execution
  surface, which is tested in `core-tests/base/cell` and `async/spawn_*`.

## 1. Cross-stdlib usage

`core.sys.context_ops` is the canonical TLS / DI surface for everything
that lives above the V-LLSI bootstrap kernel:

| Consumer | Touches | Notes |
|---|---|---|
| `core/sys/common.vr` | `MAX_CONTEXT_SLOTS`, `CONTEXT_STACK_DEPTH` | Mirror constants — drift here surfaces as off-by-N writes to the per-thread arena. |
| `core/async/runtime.vr` | `context_provide` / `context_get` (via `using` clause) | Runtime context propagation across task boundaries. |
| `core/runtime/*` | `defer_register` / `defer_run_to` | Scope-exit cleanup. |

No anti-patterns surfaced. Every consumer routes through the canonical
re-export point.

## 2. Crate-side hardcodes

| Site | What it pins | Status |
|---|---|---|
| `crates/verum_vbc/src/intrinsics/runtime.rs` | `__ctx_get_raw` / `__ctx_provide_raw` / `__ctx_end_raw` / `__defer_*_raw` opcode table | OK |
| `crates/verum_context/src/lib.rs` | Per-type-id context HashMap and the per-thread arena | OK |
| `crates/verum_codegen/src/llvm/ffi.rs` | LLVM lowering of the same intrinsics for AOT | OK |

## 3. Language-implementation gaps surfaced by this suite

### 3.1 Stale `super.raw.*` mount (CLOSED in this branch)

* **Symptom (pre-fix)**: `mount super.raw.*` at `core/sys/context_ops.vr:17`
  pointed at a `core/sys/raw.vr` file that was migrated away to
  `core/intrinsics/runtime/os.vr`. The dangling mount silently
  resolved to nothing and every `__ctx_*_raw` / `__defer_*_raw` call
  failed function-id lookup at codegen time. The surrounding wrappers
  (`tls_get` / `tls_set` / `context_provide` / `context_get` /
  `context_end` / `defer_*`) all compiled to lenient panic-stubs that
  fired at runtime with `undefined function: __ctx_*_raw`.
* **Architectural class**: Same as the one already closed for
  `time_ops.vr` (see `core/sys/mod.vr` lines 39-49 for the
  migration note). The fix is mechanical: replace `mount super.raw.*`
  with the canonical `mount core.intrinsics.runtime.os.{...}` with
  the explicit list of raw intrinsics this module needs.
* **Status**: **CLOSED** in this branch. Pinned by `regression_test.vr` §A
  (three tests — TLS round-trip, context provide/get round-trip,
  defer_depth returns Int).
* **Sister files in the same class**: `file_ops.vr`, `process_ops.vr`,
  `net_ops.vr` — fixed in the same branch via the same mechanical
  replacement. Each carries the same architectural-defect pin in its
  own regression_test.vr.

### 3.2 Selective re-export resolution (already closed at the parent layer)

* **Symptom (pre-#FUNDAMENTAL)**: `mount core.sys.context_ops.{TLS_SLOT_COUNT}`
  could fail to resolve the constant because the parent-prefix scan in
  `process_import_tree` was missing.
* **Status**: **closed** by the parent-prefix scan landed in
  `vbc/codegen/mod.rs::process_import_tree` (see
  `core-tests/sys/common/audit.md` §3.1 for the canonical writeup).
  Re-pinned here by `regression_test.vr` §B to guard against re-regression
  specifically through the `context_ops` re-export path.

## 4. Action items landed in this branch

1. **Fundamental fix**: replaced `mount super.raw.*` with
   `mount core.intrinsics.runtime.os.{__ctx_*_raw, __defer_*_raw}` in
   `core/sys/context_ops.vr`. Same surgery applied in parallel to
   `file_ops.vr`, `process_ops.vr`, and `net_ops.vr`.
2. `unit_test.vr` — 14 `@test`s covering the wrapper surface
   end-to-end.
3. `property_test.vr` — 8 algebraic-law `@test`s pinning slot
   independence, write-overwrite semantics, and the per-thread arena
   ceiling.
4. `integration_test.vr` — 5 `@test`s composing the V-LLSI primitives
   with Maybe / List / fold patterns user code actually reaches for.
5. `regression_test.vr` — 4 `@test`s pinning the closed
   stale-super-raw mount defect (§A) and the selective re-export
   resolution defect (§B).

## 5. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Full `defer_register` / `defer_execute` / `defer_run_to` lifecycle | Requires composing with the closure-execution surface; tested in `core-tests/async/spawn_*` where the right tier is wired up. |
| 2 | `ContextSlots` record (`active_bitmap: Int`, `slots: [Int; MAX_CONTEXT_SLOTS]`) on top of the raw ctx_* API | Requires runtime fixture setup. Same deferral as `core-tests/sys/common/audit.md` §A. |
