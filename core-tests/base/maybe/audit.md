# Audit — `core/base/maybe.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/maybe.vr` (747 lines) |
| Tests | `core-tests/base/maybe/` — `unit_test.vr` (1402 LOC, migrated), `property_test.vr` (NEW, ~330 LOC, monad/functor/Eq/Ord laws + @property + @test_case), `integration_test.vr` (NEW, ~210 LOC, Map/Set/sort/?-chains/Result-conversion) |
| Hardcodes in `crates/` | 4 sites (Some/None tags in codegen builtin registry, ?-fast-path, FromResidual blanket impl, FxHash variant tag) |

## §1  Variant-tag drift surface

`Maybe.None = 0`, `Maybe.Some = 1` are hardcoded in the codegen
builtins registry:

```text
crates/verum_vbc/src/codegen/mod.rs:4941   ("Maybe", "None", 0, 0, vec![]),
crates/verum_vbc/src/codegen/mod.rs:4942   ("Maybe", "Some", 1, 1, vec!["_0".into()]),
```

Plus various `0x8000+0` / `0x8000+1` TypeId references in
`verum_vbc/src/interpreter/dispatch_table/handlers/{ffi_extended,arith_extended}.rs`
(see grep output in the broader audit below).

**Drift class:** identical to Ordering — if `core/base/maybe.vr` ever
re-orders the variants (e.g. `type Maybe<T> is Some(T) | None;`), the
codegen and runtime would silently disagree on tags.

**Action item (deferred):** mirror the `ORDERING_VARIANT_LAYOUT`
pattern. Add `MAYBE_VARIANT_LAYOUT: &[(&str, u32)] = &[("None", 0),
("Some", 1)]` in `verum_common/src/well_known_types.rs`. Have codegen
+ all runtime sites (`make_maybe_none()`, `make_maybe_some()`,
`pattern_matching` path) consult it. Add a unit test pinning the
matrix.

**Scope:** ~80 LOC in well_known_types + 2-3 sites in verum_vbc to
update; mechanical.

## §2  ?-operator fast-path bypasses Try / FromResidual

This is the same architectural defect as documented in
`core-tests/base/ops/audit.md §1`:

> `crates/verum_vbc/src/codegen/expressions.rs:13134-13219`
> (`compile_try()`) hardcodes Maybe/Result fast-paths bypassing the
> protocol layer entirely.

The `Try for Maybe<T>` impl at `core/base/maybe.vr:574-589` is correct
on paper but **never invoked at the codegen level** for Maybe values.
Similarly for the cross-type `FromResidual` impls at lines 591-612.

This means if you change `Maybe.branch()`'s body to do something
custom (e.g. logging on None), the change is silently ignored by the
?-operator. It's a violation of "what you read is what runs".

**Action item (deferred):** see ops audit §1 for the architectural fix.
Cross-cutting, multi-PR.

## §3  Cross-stdlib usage

Maybe is *the* most-used stdlib type. Surveying `core/`:

- Every fallible-but-not-error API returns `Maybe<T>` (e.g.
  `List.get(i)`, `Map.get(k)`, `Iterator.next()`).
- `&dyn Describable` chain in `error.vr:340` walks `Maybe`.
- `core/base/cell.vr::OnceCell` carries `Maybe<T>` inner state.

Idiomatic patterns observed:
- `?`-propagation chains (when allowed by codegen — see §2)
- `.map(...)` for transformations
- `.unwrap_or(default)` for fallback
- `if let Some(v) = ...` pattern destructure

Anti-patterns observed: none significant in the spot-checked files.

## §4  `Default` for Maybe<T>

`maybe.vr` implements `Default for Maybe<T>` returning `None`. This
makes `let m: Maybe<Int> = Maybe.default()` idiomatic. Matches
expected semantics; no defects.

## §5  Action items landed in this branch

- [x]  Migrate `vcs/specs/core/core/maybe_test.vr` →
       `core-tests/base/maybe/unit_test.vr` (vtest frontmatter stripped)
- [x]  Add `property_test.vr` covering functor laws (identity,
       composition), monad laws (left/right identity, associativity),
       Eq equivalence, Ord total order with None < Some, combinator
       algebra (and/or/xor/zip), filter idempotence, flatten levels,
       plus @property and @test_case demos
- [x]  Add `integration_test.vr` covering Map.get returning Maybe,
       sorted lists with None first, ?-chains, ok_or conversions,
       Set membership, Iterator on Maybe, filter_map idiom, Default
- [x]  Add this audit document

## §6  Action items deferred (not landed)

1. **MAYBE_VARIANT_LAYOUT canonical constant** — mirror Ordering
   pattern. Small change; ~80 LOC. Should land alongside MAYBE refactor.
2. **?-operator protocol-dispatch** — see ops audit §1. Architectural;
   cross-cutting.
3. **Iterator.try_fold and similar combinators** — once §2 lands,
   port these for Maybe-returning closures.
