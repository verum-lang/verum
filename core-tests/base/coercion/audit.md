# Audit — `core/base/coercion.vr`

> The four coercion markers (`IntCoercible`, `TensorLike`, `Indexable`,
> `RangeLike`) are the *anti-pattern remediation*: before they existed,
> the typechecker hardcoded "stdlib type X coerces to Y" rules. The
> markers replace those hardcodes with implement-block discovery.

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/coercion.vr` (164 lines, 4 marker protocols, no methods) |
| Tests | NEW from-scratch — `unit_test.vr` (~120 LOC), `property_test.vr` (~140 LOC, marker laws + @property + @test_case), `integration_test.vr` (~150 LOC, real arithmetic / indexing / range scenarios) |
| Hardcodes in `crates/` | the *whole point* of this module is replacing crate-side hardcodes; one hardcode-removal site documented below |

## §1  Architectural context — the markers replace what?

The header comments in coercion.vr quote the `verum_types/src/CLAUDE.md`
rule:

> NEVER hardcode stdlib/core type knowledge in the compiler.

Pre-coercion-markers, the unifier had per-type lists like:

```rust
// hypothetical pre-marker code (removed)
fn is_tensor_like(name: &str) -> bool {
    matches!(name, "DynTensor" | "Tensor" | "Vector"
        | "Cotangent" | "Tangent")
}
```

These lists drift the moment someone adds a new tensor-shape stdlib
type — the new type silently fails to participate in
`Float ↔ Tensor<Float>` coercion until the Rust list is updated. The
markers are the discovery mechanism: the compiler queries
`type_implements_protocol(my_type, "TensorLike")` instead.

**Consumer in `crates/verum_types`:** the unifier's coercion-rule
table consults `TypeChecker::implements_protocol` for each marker.
Implement-block discovery is metadata-driven — no string lists.

## §2  Cross-stdlib usage

Survey of where the markers are implemented in `core/`:

| Marker | Implementors (typical) |
|---|---|
| `IntCoercible` | Duration, FileDesc, Port, VmAddress, time scalars, FFI handles |
| `TensorLike` | DynTensor, Tensor, Vector, Cotangent, Tangent (in `core/math/`) |
| `Indexable` | List, Slice, Range (in `core/collections/`) |
| `RangeLike` | Range, RangeInclusive, RangeFrom, RangeTo (in `core/base/iterator.vr`) |

Spot-checking `core/` for actual implementations is a follow-up
inventory task — out of scope for this audit pass.

## §3  Drift surfaces

### 3.1  Pre-marker hardcoded-name lists

If any `crates/verum_types/src/...` file *still* has a hardcoded
list of stdlib type names that should be marker-driven instead, that's
a regression. Spot-check items found:

- `verum_types/src/operator_protocols.rs` references protocol *names*
  (`"Add"`, `"Sub"`, etc.) but those are language-level operator
  protocols, not coercion rules. **Not a regression.**
- `verum_types/src/specialization_selection.rs` historically held some
  `is_collection`/`is_tensor`-style checks; today they consult
  `WellKnownType::is_collection` / `WellKnownProtocol::...`. **OK**.

If a future audit finds a name-list-style coercion gate that should
be marker-driven, this section is where to log it. Currently empty.

### 3.2  Marker zero-method invariant

By contract these protocols have ZERO methods. If anyone accidentally
adds a method to `IntCoercible`, every implementor must update — and
the discovery path may break in subtle ways (the unifier may start
demanding the method exist on the implementor's metadata).

**Action item (deferred):** stdlib-load-time check that `IntCoercible`
/ `TensorLike` / `Indexable` / `RangeLike` have empty method tables.
Mirror the `ORDERING_VARIANT_LAYOUT` validator pattern.

## §4  Action items landed in this branch

- [x]  Scaffold `core-tests/base/coercion/` (no prior tests)
- [x]  Write `unit_test.vr` covering each marker's declared-and-
       implemented-on-user-type contract; multi-marker stack on a
       single type; generic-marker implementation
- [x]  Write `property_test.vr` covering reflexive identity coercion,
       IntCoercible round-trip, TensorLike scalar absorption,
       Indexable bounds, RangeLike preservation; plus @property
       generators and @test_case truth tables
- [x]  Write `integration_test.vr` covering real flows: counter
       round-trip, Vec2 scaling, CircularBuf wrapping arithmetic,
       Slice semantics, Address arithmetic with stacked markers
- [x]  Add this audit document

## §5  Action items deferred (not landed)

1. **Marker zero-method invariant validator** — stdlib-load-time check
   that the four markers' method tables are empty. Small Rust-side
   check; ~20 LOC in well_known_types or a new validator module.
2. **Inventory pass over `core/`** — actually count which stdlib types
   implement each marker; check for missing implementations on tensor /
   range / index families that would benefit. Pure-discovery work; no
   code changes needed up front.
3. **Negative-coercion test** — verify the unifier *refuses* coercion
   for types that DON'T implement a marker. Today our tests only
   verify the positive direction. Negative tests need
   `// @expected-error: E400` semantics that aren't natively part of
   `verum test` (only of `vtest`); deferred until the testing
   infrastructure unifies (CAPABILITY_GAPS.md).
