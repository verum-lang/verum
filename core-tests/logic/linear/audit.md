# `logic/linear` audit

Module: `core/logic/linear.vr` (540 LOC) — linear logic formula
representation + classification predicates.

LinForm 12-variant ADT covers:
* Multiplicative: ⊗ (MTensor — note: NOT bare `Tensor` to avoid
  collision with `core.math.tensor.Tensor`), ⅋ (Par), 1 (One),
  ⊥ (Bottom)
* Additive: & (With), ⊕ (Plus), ⊤ (Top), 0 (Zero)
* Exponentials: ! (OfCourse), ? (WhyNot)
* Atomic + Dual

Smart constructors: lin_atom / lin_tensor / lin_par / lin_with /
lin_plus / lin_of_course / lin_why_not / lin_dual / lin_lolli
(A ⊸ B := A^⊥ ⅋ B).

Classification: is_unrestricted (OfCourse / WhyNot / Top),
is_weakenable.

Tests: `unit_test.vr` (~25 unit tests over smart ctors, predicates,
variant disjointness for unit + atom variants).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `verum_smt` | session-typed protocol verification reduces to linear logic. |
| `verum_verification` | resource accounting in proof checker. |
| `core.types.qtt` | quantitative analogue of linear logic. |

## 2. Crate-side hardcodes

`MTensor` variant naming (NOT bare `Tensor`) is load-bearing —
documented at `linear.vr:62-67`. Renaming back to `Tensor` would
collide with `core.math.tensor.Tensor` under the bare-name first-
wins resolver (task #17/#39 hazard). Pin the name in audit so
future renames preserve the work-around.

## 3. Language-implementation gaps

### §3.1 Property tests deferred — De Morgan laws for linear negation

Classic LL laws:
* (A ⊗ B)^⊥ = A^⊥ ⅋ B^⊥
* (A & B)^⊥ = A^⊥ ⊕ B^⊥
* (!A)^⊥ = ?(A^⊥)
* Double dual: (A^⊥)^⊥ = A (via lin_to_nnf normalisation)

property_test.vr would express these as round-trip equivalences
via `lin_to_nnf` + `lin_eq`.

**Effort:** 1h.

### §3.2 `lin_to_nnf` + `lin_is_nnf` + `lin_eq` not unit-tested

The module ships these but the unit tests above only cover
construction + classification. Add tests for NNF normalisation
correctness across the 12 variants.

**Effort:** 1h.

### §3.3 `MTensor` rename rationale should propagate to test names

The doc-comment at `linear.vr:62-67` explains the
`MTensor != Tensor` choice. Audit links the test
`test_lin_tensor_constructor` (which verifies the variant tag
`LinForm.MTensor` matches the `lin_tensor()` smart ctor return).

### §3.4 No `LinForm.Display` impl with LL notation

Pretty-print `f"{form}"` to `A ⊗ B` (utf-8 connectives) for
debugging. Defer until classification routines mature.

## Action items landed in this branch

* `core-tests/logic/linear/unit_test.vr` — 25 unit tests over
  smart constructors + classification predicates + variant
  disjointness.
* `core-tests/logic/linear/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add property_test.vr (De Morgan laws via NNF round-trip) | this folder | 1h |
| Add unit tests for lin_to_nnf / lin_is_nnf / lin_eq | this folder | 1h |
| Add `Display` impl with UTF-8 LL connectives | `core/logic/linear.vr` | 1h |
| Sister tests for `core.logic.{kripke,separation}` | sister folders | 1 day each |
