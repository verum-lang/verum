# `control/continuation` audit

Module: `core/control/continuation.vr` (~553 LOC) — delimited
continuations (shift/reset) as first-class data + small-step
reduction semantics.

Tests: 33 unit tests covering 6-variant CcTerm construction +
predicates + free-variable analysis + capture-avoiding
substitution + 3-rule small-step reduction (β / reset-value /
shift) + cc_normalise gas budget + cc_alpha_eq.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.verify.kernel_*` | term encoding for continuation-calculus proofs. |
| `verum_types::continuation_calculus` | underlying analysis core consumes the same CcTerm shape. |
| Application research code | model handler-style effects without language-level effect machinery. |

## 2. Crate-side hardcodes

`verum_types/src/continuation_calculus.rs` and related Rust-side
analysis must agree with the variant ordering + payload shapes
here. Schema-mismatches surface as Tier-0 codegen panics in
verum_vbc when the analyser produces terms the interpreter
can't materialise.

## 3. Language-implementation gaps

### §3.1 Add property tests

* ∀t. cc_alpha_eq(t, t) — reflexivity
* ∀t. cc_normalise(t, 0) yields steps == 0
* ∀v, t. cc_substitute(t, v, cc_var(v)) =α= t
* ∀t,t'. if cc_step(t) is Stepped(t'), then cc_alpha_eq is preserved on closed subterms

**Effort:** ~1h.

### §3.2 Diverging-term gas-budget regression test

Construct `(λx. x x) (λx. x x)` (Omega combinator) and verify
`cc_normalise(omega, 100)` returns `steps == 100`. Validates that
the gas budget terminates non-terminating reduction.

**Effort:** trivial (~15 min) — pure test, no new code needed.

### §3.3 Document evaluation strategy

The current `cc_step` is left-to-right reduction order. Document
the choice in the module-level doc so callers can predict
side-effect ordering when they instrument the reducer.

## Action items landed in this branch

* `core-tests/control/continuation/unit_test.vr` — 33 unit tests
  over CcTerm 6-variant + smart ctors + predicates + free-var
  analysis + substitution + reduction + alpha-equivalence.
* `core-tests/control/continuation/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add property_test.vr (reflexivity, gas-budget, sub-id law) | this folder | 1h |
| Add diverging-term (Omega) regression test | this folder | 15 min |
| Sister tests for `core.eval.cbpv` | sister folder | covered |
| Sister tests for `core.theory_interop.coord` | sister folder | gated on Coord crate |
