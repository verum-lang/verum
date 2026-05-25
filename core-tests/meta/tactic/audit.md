# `meta/tactic` audit

Module: `core/meta/tactic.vr` (~232 LOC) — tactic metaprogramming
abstract algebra over the `MetaTerm` analysis type (quote / splice /
reflect / custom / seq / const).

Tests: 35 unit tests + 13 property-law tests over the full surface.

The module is **purely data + recursion over the algebra** with no
intrinsics and no cross-module record returns — every function
returns a `MetaTerm`, `Bool`, or `Text` and operates by pattern
matching. This makes `tactic.vr` the **cleanest data-only meta
submodule** and the most likely candidate to be cross-tier green
once the runner ergonomics improve.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `verum_types::tactic_meta` | analysis core for elaborator dispatch + reflect caching; the surface module `tactic.vr` is its user-facing data API |
| `core.proof` (planned) | proof-search tactic combinators |
| Compiler `@meta_macro` expansion (planned, V2) | quoted tactic terms desugar to `meta_quote`/`meta_splice` chains |

## 2. Crate-side hardcodes

`verum_types::tactic_meta::MetaTerm` mirrors the 6-variant
algebra in the Rust analyzer. Variant set + payload shapes
MUST agree.

* `Quote { payload: Text }`
* `Splice { inner: Heap<MetaTerm> }`
* `Reflect { goal_name: Text }`
* `Custom { name: Text, arg: Heap<MetaTerm> }`
* `Seq { first: Heap<MetaTerm>, second: Heap<MetaTerm> }`
* `Const { payload: Text }`

Drift-pinning macro suggested for `verum_types::tactic_meta::test`:
mirror the unit test's variant-disjointness assertions for the Rust
representation.

## 3. Language-implementation gaps

### §3.1 Custom-elaborator dispatch / Reflect caching are analyzer-side

`meta_normalise` deliberately stops at the surface layer; it does
not invoke registered Custom elaborators (`F` in `custom(F, arg)`)
or consult the Reflect goal cache. Those rewrites live in
`verum_types::tactic_meta` and require a live analyzer context.

Once the analyzer surfaces these via a Tier-1 context
(`TacticAnalyzer`?), property tests for *full* normalisation
(`Custom` reduces to applied result, `Reflect` reduces to cached
value) should land in this folder.

### §3.2 `references_elaborator` does not descend through `Custom`'s arg in a single test

Verified by `test_references_elaborator_nested_custom_arg` — the
recursion reaches arbitrary depth. Property covered by
`law_references_descends_into_splice` for the splice analogue.

## Action items landed in this branch

* `core-tests/meta/tactic/unit_test.vr` — 35 unit tests over
  MetaTerm 6-variant ctors + variant disjointness + ctors +
  predicates (`is_meta_value`, `references_elaborator`) +
  reduction (`beta_cancel`) + normalisation (`meta_normalise`,
  `meta_is_normal`).
* `core-tests/meta/tactic/property_test.vr` — 13 algebraic laws:
  β-cancellation, beta-idempotence, normalise-idempotence,
  collapse-double-splice, seq-with-first-value-reduces,
  references-monotone-under-seq, references-descends-splice,
  is_meta_value head-surface.
* `core-tests/meta/tactic/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Full normalisation tests with live Custom elaborator + Reflect cache (§3.1) | this folder + verum_types/tactic_meta | 2-3 days |
| Drift-pinning Rust unit test mirroring the variant-disjointness assertion | crates/verum_types/src/tactic_meta.rs | 30 min |
| Property test: `references_elaborator` is **closed under** `meta_normalise` — if `n = meta_normalise(t)` then `references_elaborator(t, name) ⇔ references_elaborator(n, name)` (β-cancel doesn't drop Custom calls) | this folder | 1 h |
