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

### §3.1 `validate_epsilon_canonicalisable` body is empty — CLOSED 2026-05-25

Originally documented: the loop body never updated `all_admissible`,
so R2 returned `Maybe.None` for every input.

**Closed by porting the per-Char ordinal-grammar predicate**:

```verum
fn ordinal_char_admissible(ch: Char) -> Bool {
    if ch >= '0' && ch <= '9' { return true; }   // ASCII digits
    if ch == '+'              { return true; }   // ordinal addition
    if ch == 'ω'              { return true; }   // U+03C9
    if ch == 'Ω'              { return true; }   // U+03A9
    if ch == '·'              { return true; }   // U+00B7 (ordinal mult)
    if ch == '²'              { return true; }   // U+00B2 (omega-squared)
    false
}
```

The outer fn iterates `s.chars()` and rejects the string if **any**
char is non-admissible. Empty string is explicitly inadmissible.
Primitive `ε_` / `epsilon_` prefix still trusts the AST
canonicaliser to vet the suffix.

Anchor: 15 tests in `unit_test.vr` Section 8 cover canonical
forms (`ω`, `Ω`, `ω+1`, `ω·2`, `ω²`, `ω·3+2`, `0`) and
inadmissible inputs (empty string, letter, space, minus sign,
garbage). Every inadmissible case emits an R2 Warning with
the correct rule/severity fields.

### §3.2 R4 / R5 / R6 not yet implemented

Header lists three future rules:
* R4 — `ν = e ∘ ε` (Corollary 5.10) cross-check
* R5 — citation field non-empty + Unicode-clean
* R6 — universe-level annotation matches stack_model expectation

When they land, drop sister tests mirroring §5 / §7's pattern.

## Action items landed in this branch

* `core/meta/framework_hygiene.vr` — ported the ordinal-grammar
  predicate (`ordinal_char_admissible`) and per-Char loop in
  `validate_epsilon_canonicalisable`. Closes §3.1.
* `core-tests/meta/framework_hygiene/unit_test.vr` — 39 unit tests
  (was 27): + 12 new R2 tests in Section 8 covering canonical
  ordinal forms (`ω`, `Ω`, `0`, `ω+1`, `ω·2`, `ω²`, `ω·3+2`) +
  inadmissible inputs (empty / letter / space / minus / garbage).
* `core-tests/meta/framework_hygiene/audit.md` — this file.

### §3.2 META-HYGIENE-VALIDATE-1 — 2 unit tests misused `List.append` — CLOSED 2026-07-06

`MetaClassifierTally { candidates: List.of(t1).append(List.of(t2)) }`
— `append` returns `()`; `validate_meta_classifier_uniqueness`
null-derefed indexing the Unit (`NullPointerAt opcode 0x66 … pc=4`).
Rewritten to list literals. Language hole closed by
META-TEST-TYPECHECK-1 (see meta/oracle audit §3.4).

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| R4 / R5 / R6 implementation + tests | core/meta/framework_hygiene.vr + this folder | 2-3 days |
| Integration test: `run_all_hygiene_rules` over a 5-decl batch | this folder | 30 min |
| Drift-pinning Rust unit test for brand-prefix list | crates/verum_compiler/src/audit_walker.rs | 30 min |
