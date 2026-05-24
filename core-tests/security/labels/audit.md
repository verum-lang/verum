# `security/labels` audit

Module: `core/security/labels.vr` (150 LOC) — security labels for
information-flow control via the canonical chain Public ⊑ Internal
⊑ Secret ⊑ TopSecret plus user-defined Custom labels.

Tests: `unit_test.vr` (~37 unit tests covering Label 5-variant +
labeled() ctor + flows_to lattice ordering [reflexivity, canonical
chain, downward rejection, Custom semantics] + join least upper
bound).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `verum_types.modal_types` (compiler-side) | source of truth for the modal-types analysis. |
| `core.security.{aead,cipher,encrypt,jwt,hpke,cose}` | sensitive payloads labeled at the API boundary. |
| Application info-flow code | `let secret: Labeled<...> = labeled(Label.Secret, ...)` discipline. |

## 2. Crate-side hardcodes

`crates/verum_types/src/modal_types/lattice.rs` (when implemented)
must agree with `flows_to` + `join` semantics. Drift between
compiler-side static check and runtime predicate → soundness gap.

## 3. Language-implementation gaps

### §3.1 No `Label.Eq` / `Display` / `Debug` impls

`Map<Label, Permission>` lookups need Eq. `f"{label}"` won't
compile. Add Display rendering (`"Public"` / `"Custom(FINANCE)"`)
+ Eq + Hash.

**Effort:** small (~30 min) — 5 variants.

### §3.2 `combine` API takes `op: fn(T, U) -> T` — asymmetric

`combine<T, U>(a, b, op)` returns `Labeled<T>`. The `T, U`
asymmetry forces callers into a pattern where the LEFT type wins.
A more general `combine_with<T, U, R>(..., op: fn(T, U) -> R) ->
Labeled<R>` would be symmetric.

**Effort:** small (~30 min) + 2 tests.

### §3.3 Property tests deferred — flows_to is a partial order

Laws to verify in property_test.vr:
* reflexivity: ∀x. flows_to(x, x)  [tested per-variant in unit]
* antisymmetry: flows_to(a, b) ∧ flows_to(b, a) ⇒ a == b
* transitivity: flows_to(a, b) ∧ flows_to(b, c) ⇒ flows_to(a, c)
* join is associative: join(join(a, b), c) = join(a, join(b, c))
* join is commutative: join(a, b) = join(b, a)
* join is idempotent: join(a, a) = a
* join is least upper bound: flows_to(a, join(a, b)) ∧ flows_to(b, join(a, b))

**Effort:** 2h with Label Eq impl + property tests.

### §3.4 Custom labels join to TopSecret — documented but surprising

`join(Custom("A"), Custom("B")) = TopSecret` per the conservative
upper-bound contract. May surprise callers who expected
`Custom(disjoint)`. Document prominently OR add a strict variant
that returns Maybe<Label>.

## Action items landed in this branch

* `core-tests/security/labels/unit_test.vr` — 37 unit tests over
  Label 5-variant + Labeled + flows_to (reflexivity + canonical
  chain + downward rejection + Custom semantics) + join (LUB +
  commutativity + Custom fallback).
* `core-tests/security/labels/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add Eq/Hash/Display/Debug for Label | `core/security/labels.vr` + tests | 30 min |
| Add property_test.vr (antisymmetry, transitivity, join LUB laws) | this folder | 2h |
| Add combine_with<T, U, R> general variant | `core/security/labels.vr` + tests | 30 min |
| Add `join_strict` returning Maybe<Label> for Custom-disjoint | `core/security/labels.vr` + tests | 30 min |
| Sister tests for `core.security.{aead,cipher,encrypt,jwt,hash,kdf}` | sister folders | 1 week total |
EOF
