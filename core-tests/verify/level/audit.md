# `verify/level` audit

Module: `core/verify/level.vr` (~270 LOC) — VVA §2.3 nine-strategy
verification ladder mirror of Rust-side
`verum_ast::attr::VerificationMode`.

Tests: 41 unit tests covering VerificationLevel 12-variant +
parse_level (canonical-string + unknown-input None + case-
sensitivity) + to_annotation distinctness + ν-ordinal projection
(nu_omega_coeff + nu_finite_offset matching VVA §2.3 table) +
capability predicates (requires_smt = Runtime-only-no +
allows_runtime_fallback = lax-half-yes).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.verify.attempt` | evaluate_attempt routes per-level dispatch. |
| `verum_ast::attr` | Rust-side mirror VerificationMode. |
| `@verify(<strategy>)` attribute | annotation parsing. |
| `verum audit --coord` | nu_render plain output. |

## 2. Crate-side hardcodes

* ν-ordinal table (nu_omega_coeff + nu_finite_offset) MUST agree
  with VVA §2.3 spec exactly — pinned per-variant in this branch's
  tests. Drift here changes verification-strategy semantics.
* The 12-variant set MUST agree with VerificationMode in Rust;
  M4.E added Coherent / CoherentStatic / CoherentRuntime to
  close pre-M4.E stdlib drift.

## 3. Language-implementation gaps

### §3.1 Property tests on ν monotonicity

∀l1, l2: VerificationLevel. l1 = "weaker" ⟹ nu(l1) < nu(l2) in
the ordinal sense (Runtime < Static < Fast < Formal < Proof <
Thorough < Reliable < Certified < CoherentStatic <
CoherentRuntime < Coherent < Synthesize).

**Effort:** ~30 min.

### §3.2 emits_certificate test

Module exposes `emits_certificate` — gated tests on which levels
produce CoreTerm certificates (Proof / Reliable / Certified /
Synthesize) vs. which don't (Runtime / Static / Fast / Formal /
Thorough / Coherent triple).

### §3.3 nu_render formatted output

`nu_render` produces "0" / "2" / "ω" / "ω·2+1" / "ω·2+5" / etc.
Pin each variant's exact text.

## Action items landed in this branch

* `core-tests/verify/level/unit_test.vr` — 41 unit tests over
  VerificationLevel + parse_level + to_annotation +
  nu_omega_coeff + nu_finite_offset + requires_smt +
  allows_runtime_fallback.
* `core-tests/verify/level/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Property test on ν monotonicity | this folder | 30 min |
| emits_certificate exhaustive sweep | this folder | 20 min |
| nu_render exact-string sweep | this folder | 30 min |
| Sister tests for `core.verify.{attempt,certificate,coherence}` | sister folders | 2 days total |
