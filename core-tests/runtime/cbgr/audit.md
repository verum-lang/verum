# `runtime/cbgr` audit

Module: `core/runtime/cbgr.vr` (22 LOC) — VBC-runtime CBGR (Capability-Based
Generation Reference) intrinsic forward-declarations.  Four functions:
`cbgr_generation` / `cbgr_epoch` / `cbgr_check` / `cbgr_invalidate`.

Tests: 10 unit tests covering surface + return-shape + monotonicity
invariants.  7/10 GREEN under `--interp`; 3 `@ignore` pinned on §A.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.runtime.env.ExecutionTier` | dispatches the CBGR check path per tier (Tier0_Full / Tier1_Epoch / Tier2_Gen / Tier3_Unchecked); `.overhead_ns` pins the 15/8/3/0 ns cost contract. |
| `core.mem.alloc` (CBGR-tracked allocator) | every allocation stamps a (gen, epoch) pair into the ThinRef header (`crates/verum_vbc/src/value.rs:1603`). |
| `crates/verum_vbc/src/interpreter/state.rs:634` (`cbgr_epoch: u64`) | interpreter-side authoritative epoch counter. |
| `crates/verum_codegen/src/llvm/instruction.rs:18642` (`verum_cbgr_check`) | AOT-side C-fallback dispatched from inline check IR. |

The user-side `core.runtime.cbgr.*` surface is the **language-level**
view of these; the actual mechanism lives inside the interpreter /
codegen.  Drift between the language surface and the runtime state
(per §A below) is a soundness hazard.

## 2. Crate-side hardcodes

| Constant / site | What it pins | Risk |
|---|---|---|
| `ThinRef::generation` offset (`value.rs:1603-1612`) | 16-byte ThinRef layout; verum_cbgr_check IR reads at this offset | Drift here silently mis-classifies refs as invalid. |
| `state.cbgr_epoch` (`state.rs:634`) | u64 monotone counter; init to 1, reset to 1 on full-state reset | A reset path that forgets to reset to 1 yields off-by-one in every check. |
| AOT `verum_cbgr_check` ABI (`instruction.rs:18642`) | `fn(void*) -> i32`; loads ThinRef fields, calls `validate_ref` C-fallback | ABI drift between LLVM lowering and the bound C symbol surfaces as link errors / silent miscompiles. |

## 3. Language-implementation gaps

### §A — cbgr_* intrinsics not dispatch-bound at the user-surface level

Surfaced 2026-05-27 via `test_cbgr_check_fresh_pair_valid` failure.

* Symptom: `cbgr_check(g, e)` returns `false` for a freshly-read
  `(g, e) = (cbgr_generation(), cbgr_epoch())` pair.
* Diagnosis: the `@intrinsic("verum.runtime.cbgr_*")` ident strings
  forward-declared at `core/runtime/cbgr.vr` are NOT registered in
  `crates/verum_vbc/src/interpreter/dispatch_table/` nor in
  `crates/verum_codegen/src/llvm/instruction.rs`.  The implementation
  exists only at the **inline-check** level — every ThinRef
  dereference compiles to a `verum_cbgr_check(ptr)` call (AOT) or an
  inline state-comparison (interp) keyed on the per-allocation header
  fields, not on the user-side `cbgr_*` free functions.
* Net effect: the four free functions form a **dead surface** —
  callable (every call returns 0/false) but semantically inert.  This
  is dangerous: a developer writing `if cbgr_check(g, e) { ... }`
  expecting the cost-model contract gets always-false, silently
  skipping the protected path.

Fix surface (multi-day VBC + codegen work):

1. Register the four idents in
   `crates/verum_vbc/src/interpreter/intrinsic_registry/` (or
   equivalent).  `cbgr_generation` → `state.cbgr_epoch as i64`,
   `cbgr_epoch` → same, `cbgr_check(g, e)` → compare against
   current state, `cbgr_invalidate(g)` → bump epoch.
2. Mirror in `crates/verum_codegen/src/llvm/` so the AOT path lowers
   the user-side calls into the same inline check sequence
   `verum_cbgr_check` already uses.
3. Decide whether the user-side surface is a **read-only diagnostic
   surface** (cbgr_generation/epoch return CURRENT runtime state) or
   a **per-reference probe** (cbgr_check takes a stored (gen, epoch)
   from somewhere else).  Documented contract should pick one.

Until the dispatch lands, the four functions are deprecated-in-practice;
flag for documentation as such until the wiring is real.

### §B — cbgr_check arity ambiguity vs `verum_cbgr_check`

`core/runtime/cbgr.vr` declares `cbgr_check(gen: Int, epoch: Int) -> Bool`
(2-arg, takes the gen/epoch).  `crates/verum_codegen/src/llvm/instruction.rs:18642`
declares `verum_cbgr_check(void*) -> i32` (1-arg, takes a ThinRef pointer).
These are **two different things** but the naming overlap is a hazard
class for future readers.  Audit recommendation: rename the
language-surface function to `cbgr_check_pair` and reserve `cbgr_check`
for the future ref-probe surface.

## Action items landed in this branch

* `core-tests/runtime/cbgr/unit_test.vr` — 10 unit tests; 7 GREEN
  under `--interp` (surface + monotonicity); 3 `@ignore` pinned on §A.
* `core-tests/runtime/cbgr/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| §A wire `cbgr_*` intrinsics through VBC + codegen | `crates/verum_vbc/` + `crates/verum_codegen/` | 1–2 days |
| §B rename `cbgr_check` → `cbgr_check_pair` | `core/runtime/cbgr.vr` + grep | 30 min (consult task #17/#39 collision class first) |
| Property test on monotonicity-across-async-points | this folder | gated on async-spawn under interp |
| AOT-tier validation across all 4 fns | this folder | gated on §A first |
