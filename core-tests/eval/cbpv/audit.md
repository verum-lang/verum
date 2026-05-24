# `eval/cbpv` audit

Module: `core/eval/cbpv.vr` (~680 LOC) — Call-By-Push-Value
calculus: a unified intermediate representation where every
term is tagged Value or Computation, supporting both
call-by-value and call-by-name as derived strategies.

Tests: 38 unit tests covering CbpvKind 2-variant + CbpvTerm
7-variant + smart ctors + cbpv_kind_of classification +
cbpv_is_canonical + cbpv_occurs_free + cbpv_substitute +
CbpvStep 2-variant + 3-rule small-step reduction (force-thunk /
sequence-bind / β) + cbpv_normalise + cbpv_alpha_eq.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.control.continuation` | sibling reduction calculus; shares fresh-name convention. |
| `core.verify.kernel_*` | term encoding for cbpv-style soundness proofs. |
| Application research code | translate other languages with effects to a uniform IR. |

## 2. Crate-side hardcodes

None directly today — pure-Verum implementation. Future
optimisation work in `verum_codegen` may pattern-match on CBPV
shape to emit specialised closure layouts.

## 3. Language-implementation gaps

### §3.1 Property tests

* ∀t. cbpv_alpha_eq(t, t)
* ∀t. cbpv_kind_of(t) ∈ {Value, Computation}
* ∀t. cbpv_kind_of(t) == Value ⟹ cbpv_is_canonical(t) ∨ t is Var
* ∀v. cbpv_substitute(cbpv_var(v), v, x) =α= x

**Effort:** ~1h.

### §3.2 force(thunk c) round-trip law

Demonstrate `force (thunk c) ↦* c` for every shape of c. Pin
the key CBPV identity that drives the IR's correctness.

**Effort:** small (~30 min).

### §3.3 Diverging-term gas-budget regression test

CBPV's `force (thunk (force (thunk … force (thunk c) …)))`
chain reduces but never terminates without gas. Verify the
budget bounds.

## Action items landed in this branch

* `core-tests/eval/cbpv/unit_test.vr` — 38 unit tests over
  CbpvKind 2-variant + CbpvTerm 7-variant + ctors + classifier
  + free-var analysis + substitution + reduction + alpha-equiv.
* `core-tests/eval/cbpv/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add property_test.vr (reflexivity, kind classification law) | this folder | 1h |
| Add force(thunk c) round-trip suite | this folder | 30 min |
| Sister tests for `core.control.continuation` | sister folder | covered |
