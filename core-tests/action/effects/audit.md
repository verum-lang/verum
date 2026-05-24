# `action/effects` audit

Module: `core/action/effects.vr` (~400 LOC) — Effect taxonomy +
commutativity classification for parallel composition coherence.

Defines EffectKind 8-variant + is_commutative_effect predicate +
effect_epsilon_coord (Diakrisis ε-coordinate per Theorem 17.T1) +
effect_kind_name canonical naming + parallel_coherent whole-list
gate (Corollary 17.C2).

Tests: 31 unit tests covering EffectKind variant construction +
commutativity classification + Diakrisis coordinate + canonical
name + parallel_coherent edge cases.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `verum_verification` | the `@verify(coherent)` SMT obligation consults parallel_coherent. |
| `core.action.{primitives,enactments}` | tag actions with EffectKind for verification. |
| `crates/verum_types/passes/effect_check.rs` | static effect tracking. |

## 2. Crate-side hardcodes

The 8-variant EffectKind taxonomy is load-bearing for the Diakrisis
Theorem 17.T1 proof carrier — adding a variant requires re-proving
Theorem 17.T1 + updating `parallel_coherent`. Pinned by audit.

## 3. Language-implementation gaps

### §3.1 Closed in this branch — qualified is_commutative_effect arms

Source-side fix in this round (qualified `EffectKind.<Variant>`).

### §3.2 Add Eq / Display / Debug for EffectKind

Existing source has Eq impl. Add Display rendering to
`effect_kind_name` output; Debug ditto.

**Effort:** trivial (~10 min).

### §3.3 Property test for parallel_coherent ↔ ∀i. is_commutative_effect

The whole-list predicate should be equivalent to "every element
is_commutative_effect". Pin this with a property test once
List iteration over the 8-variant domain is convenient.

**Effort:** small (~30 min).

### §3.4 Diakrisis ordinal arithmetic

The "ω + 1" string is a placeholder for the ordinal representation
— a future Diakrisis surface might want Ordinal type with strict
`<` and arithmetic. Document deferral.

## Action items landed in this branch

* `core/action/effects.vr` — qualified is_commutative_effect +
  effect_epsilon_coord + effect_kind_name match arms.
* `core-tests/action/effects/unit_test.vr` — 31 unit tests
  covering 8-variant + 3 helper functions + parallel_coherent
  with positive (empty/all-pure/all-commutative) + negative
  (State/Io/Exception) cases.
* `core-tests/action/effects/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add Display / Debug for EffectKind | `core/action/effects.vr` + tests | 15 min |
| Add property_test.vr (parallel_coherent ↔ ∀i. commutative) | this folder | 30 min |
| Add Ordinal type for ε-coordinate | `core/action/diakrisis.vr` (new) | 1 day |
| Sister tests for `core.action.{primitives,enactments,gauge,articulation,verify,monads}` | sister folders | 1 week total |
EOF
