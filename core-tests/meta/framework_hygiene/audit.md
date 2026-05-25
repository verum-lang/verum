# `meta/framework_hygiene` audit

Module: `core/meta/framework_hygiene.vr` (~305 LOC) — compile-time
discipline for `@framework(name, citation)` + `@enact(epsilon = …)`.
Three rules: R1 (foundation-neutral names), R2 (ε-coordinate
canonicalisable), R3 (meta-classifier uniqueness).

Tests: 27 unit tests covering HygieneSeverity 3-variant +
HygieneDiagnostic ctor + severity_as_text mapping +
name_has_brand_prefix (5 banned prefixes + case-sensitivity) +
validate_foundation_neutral_name (R1) +
validate_meta_classifier_uniqueness (R3) +
validate_epsilon_canonicalisable (R2 — current behaviour pin).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.math.frameworks.*` | declares `@framework(corpus, "citation")` + `@enact(epsilon = ...)` |
| `core.proof::framework_axioms` | reads R3 tally to enforce single meta-classifier |
| `verum_compiler::verify_phase` | calls `run_all_hygiene_rules` once per module + gates downstream phases on (severity == Error)-free result |
| `verum_lsp::diagnostics` | surfaces R1 / R3 warnings inline in IDE |

## 2. Crate-side hardcodes

* `verum_compiler::audit_walker::brand_prefixes` mirrors the 5
  banned prefixes (`diakrisis_*`, `actic_*`, `msfs_*`, `uhm_*`,
  `noesis_*`). A drift-pin macro should mirror the unit test
  `test_name_has_brand_prefix_{diakrisis,actic,msfs,uhm,noesis}`
  series.
* `verum_lsp::framework_lints` reads `HygieneSeverity` mapping
  to translate to LSP `DiagnosticSeverity`.

## 3. Language-implementation gaps

### §3.1 `validate_epsilon_canonicalisable` body is empty (medium)

```verum
let mut i: Int = 0;
let mut all_admissible: Bool = true;
while i < s.len() as Int {
    // Per-byte check would go here; this uses a simpler
    // structural test (the AST canonicaliser is the source of
    // truth and emits its own diagnostic on rejection).
    i = i + 1;
}
if !all_admissible { ... }
```

The loop body never updates `all_admissible`, so every input
takes the early-return path (returning `Maybe.None` whenever
the string starts with `ε_` / `epsilon_`) or the loop's
post-condition (`all_admissible == true` always → `Maybe.None`).

**Result:** R2 is currently a **no-op** for all inputs.

The header comment justifies this by deferring to "the AST
canonicaliser is the source of truth", but the *meta-layer*
discipline file should still surface the diagnostic when called
with a non-primitive non-ordinal string at runtime.

**Fix path (~1h):** port the AST canonicaliser's predicate
(8 primitive names + `0`/`ω`/`ω+k`/`ω·n`/`ω·n+k`/`ω²`/`Ω`
ordinal grammar) to this function.

**Pinned in `unit_test.vr` Section 8** as the *current behaviour*
(`Maybe.None` for ordinal inputs too); once the loop body lands,
those tests should flip and the audit deferral closes.

### §3.2 R4 / R5 / R6 not yet implemented

Header lists three future rules:
* R4 — `ν = e ∘ ε` (Corollary 5.10) cross-check
* R5 — citation field non-empty + Unicode-clean
* R6 — universe-level annotation matches stack_model expectation

When they land, drop sister tests mirroring §5 / §7's pattern.

## Action items landed in this branch

* `core-tests/meta/framework_hygiene/unit_test.vr` — 27 unit tests
  covering HygieneSeverity + HygieneDiagnostic + severity_as_text +
  name_has_brand_prefix + validate_foundation_neutral_name +
  validate_meta_classifier_uniqueness + R2 current-behaviour pin.
* `core-tests/meta/framework_hygiene/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Fill in `validate_epsilon_canonicalisable` body (§3.1) | core/meta/framework_hygiene.vr | 1 h |
| Flip R2 tests from "current-behaviour pin" to true validation | this folder | 30 min after §3.1 |
| R4 / R5 / R6 implementation + tests | core/meta/framework_hygiene.vr + this folder | 2-3 days |
| Integration test: `run_all_hygiene_rules` over a 5-decl batch | this folder | 30 min |
| Drift-pinning Rust unit test for brand-prefix list | crates/verum_compiler/src/audit_walker.rs | 30 min |
