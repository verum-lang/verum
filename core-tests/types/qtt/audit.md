# `types/qtt` audit

Module: `core/types/qtt.vr` (475 LOC) — Quantitative Type Theory
quantities. Tracks runtime usage count of each binding via a
4-element lattice (Zero / One / Many / AtMost(n)) with saturation
at QTT_SATURATION = 2^30.

Tests: `unit_test.vr` (~30 unit tests covering construction +
disjointness + normalize + allows + add identity/absorbing +
mul identity/absorbing + eq).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `verum_types` (compiler-side) | tracks per-binding quantity for substructural type-checking. |
| `core.architecture.types.Capability` | quantity-aware capability lifetimes. |
| `verum_verification` | quantity constraints feed into SMT obligations. |

## 2. Crate-side hardcodes

`crates/verum_types/src/passes/qtt_check.rs` (when implemented)
must agree with the lattice + add/mul/allows semantics. Drift
between Rust-side check and Verum-side semantics → soundness gap.

## 3. Language-implementation gaps

### §3.1 Property tests deferred

Algebraic laws to verify in property_test.vr:
* add associativity: (a + b) + c == a + (b + c)
* add commutativity: a + b == b + a
* mul associativity: (a * b) * c == a * (b * c)
* mul commutativity: a * b == b * a
* mul distributes over add: a * (b + c) == (a*b) + (a*c)
* is_sub is reflexive: ∀q. is_sub(q, q)
* is_sub is transitive: a≤b ∧ b≤c ⇒ a≤c

**Effort:** 2h to write property tests over the 4-variant lattice
+ AtMost boundary cases.

### §3.2 Saturation behaviour not fully exhausted

Tests cover normalize → Many at QTT_SATURATION+1 but the
add_quantity / mul_quantity saturation boundaries (AtMost(2^30) +
AtMost(2^30) → Many) deserve dedicated boundary tests.

**Effort:** small (~30 min).

### §3.3 No `Quantity.Display` / `Debug` impls

`f"{quantity}"` would fail. Add for "[Zero]" / "[1]" / "[Many]" /
"[≤N]" canonical short-forms. Useful for error messages from
the QTT checker.

**Effort:** small (~30 min).

### §3.4 AtMost(1) intentionally NOT normalised to One — documented

The comment at qtt.vr:57-60 documents this asymmetry. Pin the
contract with a regression test: AtMost(1) allows(0)==true,
One allows(0)==false. Covered by unit_test allows section.

## Action items landed in this branch

* `core-tests/types/qtt/unit_test.vr` — 30 unit tests over Quantity
  variants + normalize + allows + add/mul identity + absorbing +
  quantity_eq matrix.
* `core-tests/types/qtt/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add property_test.vr (add/mul commutativity, associativity, distribution, is_sub reflexive/transitive) | this folder | 2h |
| Saturation boundary tests at QTT_SATURATION | this folder | 30 min |
| Add `Display` / `Debug` impls for Quantity | `core/types/qtt.vr` + tests | 30 min |
| Sister tests for `core.types.{poly_kinds,two_level}` | sister folders | 1 day each |
