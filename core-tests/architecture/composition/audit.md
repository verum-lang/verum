# `architecture/composition` audit

Module: `core/architecture/composition.vr` (~128 LOC) — ATS-V
composition algebra surface mirror.

Tests: 6 unit tests covering CompositionResult Rejected variant
+ composition_result_is_composed + composition_result_tag (stable
"composed" / "rejected" strings) + composition_result_violation_count
on empty Rejected. Composed variant tests deferred (require Shape
construction, gated on `core.architecture.types` test scaffold).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `verum_kernel::arch_composition` | authoritative composition engine. |
| `core.architecture.phase` | composition step results aggregate here. |
| ATS-V phase orchestrator | per-step diagnostic strings. |

## 2. Crate-side hardcodes

* `composition_result_tag` strings ("composed", "rejected") MUST
  agree with audit JSON output schema.
* Variant semantics: `composition_result_is_composed(Composed(_)) =
  true`; `composition_result_is_composed(Rejected(_)) = false`.

## 3. Language-implementation gaps

### §3.1 Composed-variant tests

Require Shape construction. Sister-test for
`core.architecture.types::Shape` must land first; then test
`composition_result_is_composed(Composed(shape)) = true` and
`composition_result_violation_count(Composed(_)) = 0`.

**Effort:** ~1h once Shape test scaffold exists.

### §3.2 Associativity property test

`kernel_arch_composition_associative` is an axiom — the kernel-
side property test (random Shape triples) is at
`crates/verum_kernel/src/arch_composition.rs`. A Verum-side
mirror property test would need axiom evaluation.

### §3.3 Sister tests for `core.architecture.{adjunction,
counterfactual,mtac,types,anti_patterns,capability_ontology,
yoneda,corpus}`

Each needs its own conformance suite. Multi-week.

## Action items landed in this branch

* `core-tests/architecture/composition/unit_test.vr` — 6 unit
  tests over CompositionResult Rejected variant + helpers.
* `core-tests/architecture/composition/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Composed-variant tests | this folder | 1h once Shape scaffold lands |
| Associativity property test | this folder | gated on kernel axiom eval |
| Sister tests for 8 architecture/* modules | sister folders | 2 weeks total |
