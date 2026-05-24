# `action/articulation` audit

Module: `core/action/articulation.vr` (~109 LOC) — OC-side
counterpart of Enactment (VVA §11.3 α ⊣ ε adjunction).

Tests: 18 unit tests covering Articulation record (framework /
citation / lineage) + articulation_new + raw_actic_articulation
+ primitive_articulation (per-Primitive lineage) +
articulation_eq + Eq protocol + articulation_as_text +
is_raw_actic.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.action.enactments` | Enactment.articulation: Articulation |
| `core.action.gauge` | canonicalise preserves articulation |
| `verum audit --coord` | articulation_as_text in coord rendering |
| `@enact` attribute | framework field for axiom resolution |

## 2. Crate-side hardcodes

The neutral `"actic.raw"` framework MUST agree with kernel-side
CoreTerm.FrameworkAxiom. Drift breaks audit reporting.

## 3. Language-implementation gaps

### §3.1 Display/Debug protocol tests

`Display.fmt` renders as "framework:lineage"; pin via format!().

### §3.2 Hash protocol consistency test

For all (a, b) where articulation_eq(a, b), hash(a) == hash(b).
Property test.

**Effort:** ~30 min.

### §3.3 Test articulation persistence through canonicalise

`gauge.canonicalise(e)` MUST preserve `e.articulation` per spec.
Requires Enactment construction, gated on `core.action.enactments`
test scaffold.

## Action items landed in this branch

* `core-tests/action/articulation/unit_test.vr` — 18 unit tests.
* `core-tests/action/articulation/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Display/Debug protocol tests | this folder | 30 min |
| Hash protocol consistency property test | this folder | 30 min |
| Articulation persistence through canonicalise | this folder | 1h |
| Sister tests for `core.action.{enactments,gauge,ludics,verify}` | sister folders | 1 week total |
