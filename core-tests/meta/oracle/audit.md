# `meta/oracle` audit

Module: `core/meta/oracle.vr` (~236 LOC) — LLM Oracle Tactic
(Phase D.5, experimental, `@cfg(feature = "oracle")`).
Candidate-generator-only — verified by SMT before acceptance.

Tests: 25 unit tests + 13 property-law tests over the candidate /
config / response / outcome data surface.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.meta.contexts.MetaRuntime` | hosts the actual MCP bridge that dispatches the oracle query — `oracle.vr` is the user-facing **data surface** for filtering/inspection of responses |
| `core.proof::tactics` (planned) | `proof by oracle(...)` clause routes through the helpers in this module |
| `verum_smt::candidate_loop` | the SMT-verification side that consumes `OracleCandidate.proof_term` |

## 2. Crate-side hardcodes

* `verum_compiler::tactics::oracle` mirrors the OracleConfig /
  OracleCandidate / OracleResponse / OracleOutcome shapes for the
  in-compiler dispatch path.
* `verum_runtime::mcp_bridge` (when feature-gated on) consumes
  `OracleConfig.timeout_ms` directly — the constant family
  (default 5000ms / aggressive 30000ms) lives in user-visible source
  and the bridge MUST read it from there, not redefine.

## 3. Language-implementation gaps

### §3.1 The four embedded `theorem ... proof by auto;` declarations are
not unit-test'able at the stdlib level

The bottom of `oracle.vr` has four short theorems that the
Verum verifier checks at compile time:

```verum
theorem filter_monotone_confidence(c: ..., higher: ...)
    requires c <= higher
    ensures c <= higher
    proof by auto;
```

These are **proof obligations**, not runtime tests, and are
exercised by `verum test --filter theorem::` at the compiler-test
layer. The property tests in this folder are the runtime
counterparts.

### §3.2 No regression-tests yet for boundary `confidence == 0.0` /
`confidence == 1.0` behaviour under `has_viable_candidate`

The `>=` semantics (boundary value is viable) is covered by
`test_has_viable_candidate_exact_threshold_counts_as_viable` but
not over all 4 corner-points. Property test family suggested.

### §3.3 Oracle endpoint reachability tests

The `OracleOutcome.Unavailable` / `Timeout` variants are easy to
construct (covered) but the *dispatch path* that produces them at
compile time lives in `verum_runtime::mcp_bridge` and depends on
an external endpoint. Tests for the dispatch live in a
feature-gated integration suite under `crates/verum_runtime/tests/`.

## Action items landed in this branch

* `core-tests/meta/oracle/unit_test.vr` — 25 unit tests:
  defaults / aggressive / direct construction / candidate /
  response / outcome 5-variant / has_viable / filter / count.
* `core-tests/meta/oracle/property_test.vr` — 13 laws:
  filter monotonicity, count monotonicity, count-matches-filter,
  has_viable agrees with count, default vs aggressive axes.
* `core-tests/meta/oracle/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Boundary corner-points for has_viable (§3.2) | this folder | 30 min |
| Feature-gated integration tests against a recorded LLM response (§3.3) | crates/verum_runtime/tests/ | 2-3 days |
| Cross-tier (--interp + --aot) validation | this folder | gated by AOT stdlib build (task #7) |
