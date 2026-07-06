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

### §3.4 META-ORACLE-APPEND-1 — tests misused `List.append` (returns Unit) — CLOSED 2026-07-06

19 oracle tests built candidate lists as
`List.of(c1).append(List.of(c2))` — but `List.append(&mut self,
&mut List<T>)` returns `()`. The `()` flowed into
`OracleResponse.candidates` and every consumer null-derefed
(`NullPointerAt opcode 0x66 in filter_by_confidence`) or panicked
(`method '().append' not found`). Two-layer close-out:

* tests rewritten to canonical list literals `[c1, c2, c3]`;
* stdlib gained the missing expression-position API:
  `List.concat(mut self, mut other) -> List<T>`
  (`core/collections/list.vr`) — the value-returning companion to
  `append`, mirroring `Text.concat`.

The deeper language finding: `verum test --interp` compiled the
type error without complaint because the interp/property harness
ran NO type checker at all (`compile_module_with_stdlib` has no
`verum_types` phase). Closed by META-TEST-TYPECHECK-1 — the
harness now routes each standalone test file through the same
`run_check_only` entry `verum check` uses (escape hatch:
`VERUM_TEST_LENIENT_TYPES=1`).

### §3.5 META-REFINED-FIELD-FLOATCMP-1 — refined `Float{…}` field compares lowered to signed-int compares — CLOSED 2026-07-06

`aggressive.min_confidence < default.min_confidence` returned
`false` for 0.4 < 0.7: every compare over a record field typed
`Float{>= 0.0, <= 1.0}` bit-compared the raw IEEE-754 patterns as
i64. Three legs, all closed in verum_vbc/verum_common:

1. `extract_type_name_from_ast` had no `Refined` arm — field-type
   names were stored as a truncated debug dump
   (`"Refined { base: Typ"`), so int/float classification failed
   for locally-declared refined fields.
2. The canonical `well_known_types::type_names` classifiers now
   strip refinement suffixes (`strip_refinement`): `Float{…}`
   classifies as Float everywhere.
3. `resolve_field_type_ref` had no `Refined` arm — the BAKED
   archive descriptor carried no usable TypeRef for refined fields,
   so archive-loaded `OracleConfig.min_confidence` failed
   classification even after (1)+(2). Refinement predicates are
   unaffected (assert emission reads the AST, not descriptors).

## Action items landed in this branch

* `core-tests/meta/oracle/unit_test.vr` — 25 unit tests:
  defaults / aggressive / direct construction / candidate /
  response / outcome 5-variant / has_viable / filter / count —
  candidate lists rebuilt as list literals (§3.4).
* `core-tests/meta/oracle/property_test.vr` — 13 laws:
  filter monotonicity, count monotonicity, count-matches-filter,
  has_viable agrees with count, default vs aggressive axes.
* `core-tests/meta/oracle/audit.md` — this file.

### §3.6 RED-TEAM — REFINE-FIELD-DYNAMIC-BYPASS-1 — OPEN

Literal out-of-range construction of refined fields is rejected at
compile time (E500 via SMT: `min_confidence: 1.5` and `-0.3` both
refuse). But a DYNAMIC value smuggles through:
`OracleConfig { min_confidence: nan_val(), … }` where
`fn nan_val() -> Float { 0.0 / 0.0 }` constructs successfully and
the field reads back NaN. Record-literal field writes carry no
runtime refinement assert (unlike params/returns — T1-F). Tracked as
REFINE-FIELD-DYNAMIC-BYPASS-1: emit `Assert` for refined fields at
construction/assignment when not statically discharged; NaN must
fail `>= 0.0`-style predicates.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Boundary corner-points for has_viable (§3.2) | this folder | 30 min |
| Feature-gated integration tests against a recorded LLM response (§3.3) | crates/verum_runtime/tests/ | 2-3 days |
| Cross-tier (--interp + --aot) validation | this folder | gated by AOT stdlib build (task #7) |
