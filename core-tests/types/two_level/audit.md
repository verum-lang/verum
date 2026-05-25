# `types/two_level` audit

Module: `core/types/two_level.vr` (~205 LOC) — Two-Level Type
Theory (2LTT). Voevodsky/ACK universe stratification into fibrant
(HoTT/cubical) and strict (UIP-holding, decidable equality) layers.

Tests: 26 unit tests + 4 algebraic-law tests over Layer 2-variant +
flows_to truth table + mix truth table + LayerVerdict 3-variant +
Eq matrix.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `verum_types::two_level` | analyzer-core mirror — every type carries a `Layer` tag computed at elaboration |
| `core.proof::tactics::univalence` | only applies in `Fibrant` universe (UIP would invalidate paths) |
| `core.proof::decidability` | only applies in `Strict` universe (UIP gives decidable Eq) |
| `verum_compiler::elaborate::stratify` | computes `layer_mix` across binder bodies |

## 2. Crate-side hardcodes

* `verum_types::two_level::Layer` mirrors the 2-variant enum.
* `verum_types::two_level::LayerVerdict` mirrors the 3-variant
  verdict ADT.
* `verum_compiler::stratify::flow_table` mirrors the 4-corner
  `layer_flows_to` truth table:

  ```
            | Fib  | Strict
  ---------+------+--------
  Fib      | true | true
  Strict   | false| true
  ```

  Drift here breaks soundness (allowing Strict → Fib would let
  UIP leak into HoTT, invalidating univalence axioms).

## 3. Language-implementation gaps

### §3.1 StratifiedUniverse + check_layer_flow tests deferred

`StratifiedUniverse { layer, level }` is a record. `stratified_fibrant(n)` /
`stratified_strict(n)` are cross-module factory fns returning records —
same cross-module record-return defect class as `meta/span` audit §3.1.

`universe_coerces_to` and `check_layer_flow` operate on these records.
Deferred to integration suite once cross-module fix lands.

### §3.2 Display / Debug rendering tests deferred

Layer / StratifiedUniverse / LayerVerdict all impl Display via `f"…"`
format strings — runs into the Text builder cross-module defect.
Deferred.

### §3.3 Refined-Int constraint `Int{>= 0}` not exercised here

`StratifiedUniverse.level: Int{>= 0}` and the `level` params on
`stratified_fibrant` / `stratified_strict` carry a refinement that
the SMT backend checks at use site. Refinement-violation tests
belong at `vcs/specs/L1-core/refinement/`.

## Action items landed in this branch

* `core-tests/types/two_level/unit_test.vr` — 30 tests:
  - Layer 2-variant + disjointness
  - Layer Eq matrix (reflexive + cross-variant)
  - layer_flows_to 4-corner truth table (Fib→Fib=true, Fib→Strict=true,
    Strict→Strict=true, Strict→Fib=false)
  - layer_mix 4-corner truth table + strict-contagion property
  - 4 algebraic laws (mix commutativity / fibrant-idempotent /
    strict-idempotent / strict-absorbing)
  - LayerVerdict 3-variant (LayerOk / LevelMismatch / StrictInFibrant)
    + disjointness
  - LayerVerdict Eq matrix (LayerOk singletons / LevelMismatch
    reflexive + from-differs / to-differs / cross-variant)
* `core-tests/types/two_level/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| StratifiedUniverse + check_layer_flow integration tests (§3.1) | this folder | 1 h after cross-module fix |
| Display / Debug rendering tests (§3.2) | this folder | 30 min after cross-module fix |
| Refined-Int `level: Int{>= 0}` boundary tests (§3.3) | vcs/specs/L1-core/refinement/ | 1 h |
| Property test: layer_flows_to is reflexive + transitive but NOT symmetric | this folder | 30 min |
| Drift-pinning Rust unit test for layer_flows_to truth table | crates/verum_types/src/two_level/tests.rs | 30 min |
