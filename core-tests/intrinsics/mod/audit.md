# `intrinsics/mod` audit

Module: `core/intrinsics/mod.vr` (~259 LOC) — the unified intrinsics
umbrella.  Zero logic: wildcard re-exports of twelve submodules + explicit
re-export lists for nested `runtime/*` items + three ambiguity-resolving
explicit lists (atomic / memory-slice / type_info).

Suite added 2026-07-15: unit (6) — resolution probes through
`mount core.intrinsics.{name}` for the EXPLICIT re-export lists.

## 0. The load-bearing finding — UMBRELLA-REEXPORT-RESOLVE-1 (task #21)

Wildcard-propagated names (`public mount arithmetic.*` → `mount
core.intrinsics.{wrapping_add}`) resolve NONDETERMINISTICALLY per run:
two runs of the same suite on one binary produced different unbound sets
(run A: `wrapping_add`/`popcnt`/`nop` E100-unbound + `sqrt` binding with an
`__opaque_src` param; run B: `saturating_add`/`rotl`/`is_nan`/`likely`
unbound).  Names on the EXPLICIT re-export lists stayed stable across both
runs — the explicit list creates a direct index entry; wildcard propagation
walks a map whose iteration order rolls dice.

Family: the bake-nondeterminism map-walk class (METADATA-DETERMINISM-1
sorted the TYPE descriptor walk; the function re-export leg was not
sorted).  Fix direction: deterministic ranked resolution (min-by-key
qualified name) in the umbrella re-export walk + explicit collision policy.

The suite therefore pins ONLY the explicit lists; the wildcard acceptance
tests are the commented block in unit_test.vr (compile-time E100 — cannot
ride as @ignore because one unbound name fails the whole merged unit).

## 1. Secondary finding — deprecated meta-fn forms return 0

`size_of<Int>()` / `align_of<Int>()` return 0 through EVERY path (umbrella
and home module) — the forms are `@deprecated` legacy per the module docs;
the canonical surface is the `T.size` type-property syntax (the type_info
suite's subject).  The umbrella test pins RESOLUTION of the re-exports and
the canonical `Int.size == 8` next to them.  The generic-arg loss is the
VBC-GENERIC-INSTANTIATION class (type_info audit #9/#10 catalogue).

## 2. Crate-side drift surfaces

* The bake name index (verum_compiler archive/metadata walk) — the
  nondeterminism home.
* `verum_types` mount resolution (`path_resolution.rs` step-0.5 mount
  consult, MOUNT-TYPE-AUTHORITY-1) — the type leg is ranked; the function
  leg needs the same treatment.

## 3. Action items

**Landed (this suite)**
* Explicit-list resolution pins (6 tests, both-tier eligible).

**Deferred (tracked)**
* UMBRELLA-REEXPORT-RESOLVE-1 (task #21) — deterministic function-name
  index; promote the acceptance block.
