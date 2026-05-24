# `action/primitives` audit

Module: `core/action/primitives.vr` (~147 LOC) — eight Diakrisis
Actic ε-primitives (Math/Compute/Observe/Prove/Decide/Translate/
Construct/Classify).

Tests: 32 unit tests covering Primitive 8-variant + canonical
ε-tag rendering + 6 classification predicates (is_observational,
is_constructive, is_proof_producing, is_decision_point,
is_translation, is_classification) + primitive_eq + Eq protocol.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.action.enactments` | Enactment.steps: List<Primitive> |
| `core.action.gauge` | canonicalisation via observational-run absorption |
| `core.action.articulation` | primitive_articulation per-Primitive |
| `@enact` attribute | epsilon = "ε_classify" tag matching |

## 2. Crate-side hardcodes

The 8-variant ε-primitive set MUST agree with Noesis NP server
+ verum_audit --epsilon output. ε_classify (8th variant) was
added per Diakrisis↔Noesis↔Verum cross-audit recommendation #3.

## 3. Language-implementation gaps

### §3.1 Display/Debug protocol tests

`Display.fmt` renders as canonical ε-tag; `Debug.fmt_debug`
renders variant name. Pin via format!() once @test allows
format-output assertion.

### §3.2 Property test — predicate exclusivity

∀p: Primitive. exactly one of {is_observational, is_proof_producing,
is_decision_point, is_translation, is_classification} is true,
EXCEPT for the 3 constructive variants (Math/Compute/Construct).

**Effort:** ~30 min.

## Action items landed in this branch

* `core-tests/action/primitives/unit_test.vr` — 32 unit tests
  over Primitive + as_text + 6 predicates + eq.
* `core-tests/action/primitives/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Display/Debug protocol tests | this folder | 30 min |
| Predicate-exclusivity property test | this folder | 30 min |
| Sister tests for `core.action.{mod,ludics,verify,enactments}` | sister folders | 1 week total |
