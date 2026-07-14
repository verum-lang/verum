# `runtime/cbgr` audit

Module: `core/runtime/cbgr.vr` — the runtime-facing CBGR shim.  Since
RUNTIME-DUPLICATE-TREE-1 (task #15) closed, the module is a thin
re-export of the canonical, WIRED declarations in
`core.intrinsics.runtime.cbgr` — one source of truth, both tiers.

**2026-07-14 — §A / §B RESOLVED, suite rewritten to the real contract.**

* §A (dead `verum.runtime.cbgr_*` intrinsic keys → always-zero stub
  surface) was closed at the root by the re-export conversion: every
  name now resolves to the live registry entries exercised green at
  `core-tests/intrinsics/runtime/cbgr/` (interp 23/23 + AOT 23/23).
* §B (arity/semantics ambiguity): the historical shim invented
  `cbgr_generation()` / `cbgr_epoch()` / `cbgr_check(gen, epoch)` —
  names and signatures that never existed canonically.  Resolution:
  NO local renames or aliases; the shim re-exports the CANONICAL
  names/signatures only (`cbgr_check(thin_ref_ptr) -> Int`,
  `cbgr_get_epoch() -> Int`, `cbgr_get_generation(ptr) -> UInt32`,
  allocation bridge, epoch transitions).  An alias layer here is
  exactly the duplicate-name drift this module was purged of.
* The re-export list was widened 2026-07-14 from {cbgr_check,
  cbgr_invalidate} to the full coherent runtime read/probe surface
  (validate, allocate/deallocate/realloc, check/check_fat/check_write,
  validate_ref, epoch reads + transitions, generation ops).

Tests: `unit_test.vr` rewritten 2026-07-14 — re-export coherence
(shim-path allocation bridge + validate), epoch stability laws,
per-allocation invalidate isolation + defensive edges.  Deep bridge
semantics (alignment sweeps, realloc preservation, extent exactness)
stay pinned at `core-tests/intrinsics/runtime/cbgr/` — not duplicated.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.runtime.env.ExecutionTier` | dispatches the CBGR check path per tier (Tier0_Full / Tier1_Epoch / Tier2_Gen / Tier3_Unchecked); `.overhead_ns` pins the 15/8/3/0 ns cost contract. |
| `core.mem.alloc` (CBGR-tracked allocator) | every allocation stamps a (gen, epoch) pair into the ThinRef header (`crates/verum_vbc/src/value.rs:1603`). |
| `crates/verum_vbc/src/interpreter/state.rs:634` (`cbgr_epoch: u64`) | interpreter-side authoritative epoch counter. |
| `crates/verum_codegen/src/llvm/instruction.rs` (`verum_cbgr_check`) | AOT-side C-fallback dispatched from inline check IR. |

## 2. Crate-side hardcodes

| Constant / site | What it pins | Risk |
|---|---|---|
| `ThinRef::generation` offset (`value.rs:1603-1612`) | 16-byte ThinRef layout; verum_cbgr_check IR reads at this offset | Drift here silently mis-classifies refs as invalid. |
| `state.cbgr_epoch` (`state.rs:634`) | u64 monotone counter; init to 1, reset to 1 on full-state reset | A reset path that forgets to reset to 1 yields off-by-one in every check. |
| AOT `verum_cbgr_check` ABI | `fn(void*) -> i32`; loads ThinRef fields, calls `validate_ref` C-fallback | ABI drift between LLVM lowering and the bound C symbol surfaces as link errors / silent miscompiles. |

## 3. Language-implementation gaps

None open at this surface.  Historical §A/§B (see header) are closed;
regressions would surface as the unit suite's "resolved to a dead
stub" assertions firing.

## Action items landed

* 2026-07-14: shim re-export list widened to the full coherent
  canonical surface (`core/runtime/cbgr.vr`).
* 2026-07-14: `unit_test.vr` rewritten from the invented-signature
  suite (3 permanently-`@ignore`d tests) to the real contract — zero
  `@ignore` remaining.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Property test on epoch-monotonicity-across-async-points | this folder | gated on async-spawn under interp |
| `cbgr_get_generation(ptr)` live probe through the shim | this folder | needs `Int -> *const Byte` cast idiom pinned first |
