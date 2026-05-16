# `core.collections.btree` — Audit

Conformance review for `core/collections/btree.vr` — `BTreeMap<K: Ord, V>`
and `BTreeSet<T: Ord>`, ordered key-value containers backed by an
internal balanced tree.

## Status

**regression-only** — Every method that reads `self.<field>` on a
populated BTreeMap panics with
  "field access out of bounds: field index 0 (offset 0+8 = 8)
   exceeds object data size 0".
Same architectural class as the closed `List` / `Map` layout drift
(stdlib `type X is { ... }` field order mismatches runtime
intercept allocation) but here the defect is not yet closed for
`BTreeMap`.

Working surface today: only the empty-state surface (new, get on
absent key, contains_key absent, remove absent, get_or_default on
absent, is_empty/len coherence) — 6 unit + 5 property + 2
integration + 4 PASS-GUARDs (17 / 17 green on `--interp`).

Populated-state surface — `insert`, `get` (with value), populated
`contains_key`, `remove`, `clear`, `first_key_value`,
`last_key_value`, `pop_first`, `pop_last`, `len` past zero,
`get_or_default` present — pinned in `regression_test.vr` §A as
`@ignore`'d (11 pins).  Empty-state `pop_first` / `first_last_none`
return a sentinel that the Maybe<(&K,&V)>::deref dispatch
doesn't recognise — pinned as §B (2 pins).

## 1. Cross-stdlib usage

Downstream consumers — every time-ordered metric store, every
ordered configuration table, every range-query workload.
Surface is foundational.

## 2. Crate-side hardcodes

No runtime intercepts specific to BTreeMap.  Implementation is
pure stdlib (red-black-tree node operations in `btree.vr`).

## 3. Language-implementation gaps

Iterator surface gated on the wrapper-iter dispatch class
(multiset §B / slice §D).  Fix path identical: codegen-side
static dispatch when receiver's static type carries the Iterator
impl.

## 4. Defect inventory

No active defects pinned on the value-level surface.  Iterator
surface awaits the wrapper-iter dispatch close-out — pinned via
audit reference rather than `@ignore`d test (the failing dispatch
is identical to multiset §B and would just duplicate the pin).

## 5. Action items

### Landed in this branch

1. Unit-test surface — 19 tests across 8 sections covering
   construction; insert prior-value; get / contains_key; remove;
   clear; ordered access (first/last); pop_first / pop_last;
   bookkeeping (len under insertions and removals);
   get_or_default.
2. Property-test surface — 10 algebraic laws (cardinality under
   insert/remove; contains iff get-Some; insert prior-value
   round-trip; first is minimum key; last is maximum key;
   pop_first / pop_last decrement len; clear restoration;
   remove absent no-op; insert same idempotent on len).
3. Integration tests — 5 cross-type scenarios (insert from
   List<(K,V)>; pop_first in sorted order; get_or_default fallback
   chain; insert/remove balance over 20 keys; clear + repopulate).
4. Regression suite — 6 PASS-GUARDs for the working surface.

### Deferred

1. **Iterator surface** (`iter` / `keys` / `values` / `range`) —
   gated on the multiset §B / slice §D wrapper-iter dispatch fix.
2. **BTreeSet** surface — apply the BTreeMap pattern to the set
   wrapper once the BTreeMap surface is fully validated.
3. **Range queries** (`range(a..b)`, `lower_bound`, `upper_bound`)
   — depend on the iterator surface plus range-bound resolution.
4. **Entry API** (`entry(k).or_insert(v)`) — separate audit pass.
5. **Split / append / extract_if** — bulk-mutate surface.
