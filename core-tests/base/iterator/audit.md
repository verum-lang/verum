# Audit — `core/base/iterator.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/iterator.vr` (4401 lines — largest in `core/base/`) |
| Tests | `core-tests/base/iterator/` — `unit_test.vr` (2150 LOC, migrated), `basic_test.vr` (262, migrated), `protocol_agnostic_test.vr` (376, migrated), `property_test.vr` (NEW, ~280 LOC, fold/map/filter/zip/range laws + @property + @test_case), `integration_test.vr` (NEW, ~200 LOC, real pipelines) |
| Hardcodes in `crates/` | Significant — Iterator is well-known protocol with specialised dispatch in fast-paths |

## §1  Iterator's role

Iterator is the most-consumed protocol after Eq/Ord. Beyond the
declared protocol methods (`next`, `size_hint`), the file defines
~50 adapter methods (`map`, `filter`, `take`, `skip`, `zip`, `chain`,
`fold`, `enumerate`, `rev`, `take_while`, `skip_while`, `peek`, …)
plus utility constructors (`once`, `repeat`, `count_from`, `from_fn`)
and the Transducer rank-2 API.

The `core/base/protocols.vr` definition of `Iterator` declares the
*minimal* protocol. The adapter methods live in `iterator.vr` because
they're stdlib code, not protocol obligations.

## §2  Drift surfaces

### 2.1  WellKnownProtocol::Iterator

`crates/verum_common/src/well_known_types.rs` registers `Iterator` in
the `WellKnownProtocol` enum. `crates/verum_types` consults this for
specialisation decisions (e.g. fast-path collection lookups). The name
must stay synchronised with `protocols.vr`'s `Iterator` declaration.

### 2.2  Transducer / Reducer rank-2 types

`iterator.vr` defines `Transducer<A, B>`, `StatefulTransducer<A, B, S>`,
`Reducer<A, R>`, `ReduceResult` — these use rank-2 polymorphic
`fn<R>(...)` types per the CLAUDE.md grammar example. Codegen for
rank-2 types is captured in `verum_vbc/src/codegen/...:Rank2Function`
opcode handling (see project memory: VBC bytecode encode/decode
symmetry, commit `5f25a7bc`).

**Drift surface:** if the transducer protocol shape changes (e.g. an
extra associated type), the codegen rank-2 lowering may need to update
in lockstep. Today this is implicit; no automated check.

### 2.3  for-loop desugaring

`for x in xs.iter() { ... }` desugars to `Iterator.next` calls.
Codegen consults the `Iterator` protocol's `next` method by name — if
`protocols.vr` ever renames `next` to `step`, every `for` loop
silently breaks until the rename propagates.

**Action item (deferred):** load-time check that
`Iterator` protocol's method table contains `next` with the expected
signature. Mirror the `primitive_protocol_matrix_pinned` pattern.

### 2.4  Iterator-item method dispatch (known type-inference defect)

`regression_test.vr::test_iterator_deref_reverse` (in `core-tests/base/ordering/`)
pins the type-inference issue:

```verum
for ord in xs.iter() {
    ord.reverse();   // FAILS: method not resolved
    (*ord).reverse();// works
}
```

Root cause is in `verum_types/src/infer.rs` — iterator-item type is
inferred too late for method-resolution. This affects every test that
iterates with method calls. Workaround documented; language fix
deferred.

## §3  Cross-stdlib usage

Iterator-driven iteration is *the* canonical idiom across `core/`:
- Every collection (`List`, `Map`, `Set`, `Deque`, `BTreeMap`,
  `BTreeSet`, `BinaryHeap`) implements `IntoIterator`.
- `Range` / `RangeInclusive` are themselves Iterators.
- Async / streaming code uses Iterator-derived stream patterns.

No idiomatic anti-patterns observed in the spot-checked files.

## §4  Action items landed in this branch

- [x]  Migrate three test files from `vcs/specs/core/core/`:
       `iterator_test.vr` → `unit_test.vr`,
       `iterator_basic_test.vr` → `basic_test.vr`,
       `iterator_protocol_agnostic_test.vr` → `protocol_agnostic_test.vr`
- [x]  Strip vtest frontmatter
- [x]  Add `property_test.vr` covering fold-empty, fold-sum identity,
       map length-preservation, filter ordering, count = fold-by-1,
       take+skip complementarity, zip parity, chain length, enumerate
       indices, sum-via-fold, any/all duality, range-count laws,
       plus @property and @test_case demos
- [x]  Add `integration_test.vr` covering filter→map→collect, product
       of evens, Map.values, Set iteration, Range arithmetic, chain,
       zip pairwise sum, enumerate, rev, take_while, skip_while,
       count_from
- [x]  Add this audit document

## §5  Action items deferred (not landed)

1. **Iterator protocol method-table validator** — load-time check that
   `Iterator.next` and other consumed methods exist with expected
   signatures. ~50 LOC.
2. **Iterator-item type-inference fix** — language-level; affects every
   for-loop that calls methods on the iterator item. Out of band for
   this audit pass; tracked in `core-tests/base/ordering/audit.md §3.1`.
3. **Transducer-protocol shape validator** — would mirror Maybe/Result
   layout pinning but for rank-2 types. Logged for future iteration
   when transducer adoption grows.
4. **Iterator.try_collect / try_fold for Result** — listed in
   `core-tests/base/result/audit.md §6` but conceptually belongs here.
   Implementation lives in iterator.vr.
