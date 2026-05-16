# `core.collections.lru` — Audit

Conformance review for `core/collections/lru.vr` — `LruCache<K, V>`,
capacity-bounded LRU.  Backed by `Map<K, V>` for O(1) lookup +
`List<K>` for LRU order tracking.

## Status

**complete** — 40/40 green on `--interp`:

* 21 unit tests across 8 sections (construction; insert prior-value;
  get/peek/contains; remove; clear; eviction; stats; peek-no-touch)
* 9 property tests (len ≤ capacity; capacity invariant; insert/remove
  inverse; stats monotone; peek no-stats-change; clear restoration;
  contains iff peek-Some; insert idempotent; prior-value round-trip)
* 5 integration scenarios (memoisation; capacity-bounded growth;
  hit/miss ratio; replace + remove; clear after population)
* 5 PASS-GUARD pins on the working surface

## 1. Cross-stdlib usage

Search across `core/` for `LruCache` consumers — surface is mostly
prospective.  Memoisation pipelines, decoded-parse caches,
request→response caches.

## 2. Crate-side hardcodes

No Rust-side runtime intercepts for LruCache; every operation
pushes through `Map<K, V>` + `List<K>` whose runtime intercepts
are tested separately.

## 3. Language-implementation gaps

| Gap | Impact | Fix path |
|---|---|---|
| `LruCache.iter()` does not exist — diagnostic enumeration uses `.stats()` instead | Cannot inspect cache contents in tests | Add `iter()` returning Map.iter()-style yields once the wrapper-iter dispatch class (multiset §B / slice §D) closes. |

## 4. Defect inventory

Per `regression_test.vr`:

* No active defects pinned on the conformance surface — the suite
  is PASS-GUARD-only.  Any future regression on insert / get /
  peek / remove / clear / contains / capacity / stats will be
  pinned as a new entry.

## 5. Action items

### Landed in this branch

1. Unit-test surface — 21 tests across 8 sections (construction;
   insert prior-value; get/peek/contains; remove; clear;
   eviction-at-capacity; stats hits/misses/evicted; peek doesn't
   touch order).
2. Property-test surface — 9 algebraic laws (len ≤ capacity;
   capacity invariant; insert/remove inverse; stats monotone;
   peek no-stats-change; clear restoration; contains iff
   peek-Some; insert idempotent under same value; insert prior-
   value round-trip).
3. Integration tests — 5 scenarios (memoise round-trip;
   capacity-bounded growth; hit/miss ratio; replace + remove;
   clear after population).
4. Regression suite — 5 PASS-GUARDs for the working surface;
   no @ignore'd defect pins (no known defects on the conformance
   surface).

### Deferred

1. `iter()` enumeration — gated on the wrapper-iter dispatch
   defect class (multiset §B / slice §D close-out).
2. Property tests over per-key LRU-order semantics — currently
   pinned via examples; once iter exists, run exhaustive
   property over insertion-order sequences.
