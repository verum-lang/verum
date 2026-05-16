# `core.collections.multiset` — Audit

Conformance review for `core/collections/multiset.vr` —
`Multiset<T: Hash + Eq>` (hash bag with strictly-positive
multiplicities backed by `Map<T, Int>`, with a cached
`cardinality` counter for O(1) total size).

## Status

**partial** — Unit, property, and integration tests cover the
working API surface:

* Construction (`new` / `with_capacity`)
* Insertion (`insert` / `insert_n` — positive, zero-no-op, negative-no-op)
* Removal (`remove` / `remove_n` saturation / `remove_all`)
* Element access (`count` / `contains` / `cardinality` / `distinct_len`)
* `is_empty` / `clear`
* Two-counter invariant (`cardinality == Σ count(k)`)
* Algebraic operations with EMPTY operand (union/intersection)
* Containment (`is_subset` early-exit + empty cases)

Residual gaps:

1. `Multiset.from([...])` / `Multiset.from_counts([...])` raise
   `UndefinedFunction` at user-side call sites (§A) — cross-module
   constructor name-table cascade.
2. `Multiset.iter()` yields no elements (§B) — wrapping `Map.iter()`
   loses dispatch.  Cascades into per-element-correct
   `union` / `intersection` / `sum` / `difference` computations
   (the with-empty cases pass because the iter body is a no-op).

## 1. Cross-stdlib usage

`Multiset<T>` is used as a frequency / histogram container.  Today the
most concrete consumer is in `core/text/word_frequency.vr` (per the
multiset module docstring), plus statistical pipelines under
`core/metrics/` and the simplicial-multiset structures referenced in
`core/cog/`.  Cross-stdlib usage is light but growing.

## 2. Crate-side hardcodes

There are no Rust-side runtime intercepts for Multiset specifically —
every operation pushes through `Map<T, Int>` whose runtime intercepts
are extensively tested in `core-tests/collections/map/audit.md`.

The implication: Multiset inherits the Map-side dispatch defects but
adds none of its own.  When a Multiset method fails (e.g.
hypothetical `cardinality()` returning a stale value), the root cause
is in `Map.get` / `Map.insert` / `Map.remove` — fixes land in
`map.vr` + `method_dispatch.rs`, not here.

## 3. Language-implementation gaps

| Gap | Impact | Fundamental fix |
|---|---|---|
| `Multiset.from([...])` / `from_counts([...])` cross-module UndefinedFunction | User-side call sites can't use the array-literal builders. | Same fix path as Tasks #24/#25/#26 close-out for general stdlib-call cross-module resolution: include the constructor function-ids in the user-side cross-module name table. |

## 4. Defect inventory

Per `regression_test.vr`:

### §A — Cross-module constructor name resolution

* §A.1 `Multiset.from([...])` raises UndefinedFunction
* §A.2 `Multiset.from_counts([...])` raises UndefinedFunction

Same architectural class as the closed cross-module Call cascade for
List / Map / Set / Deque constructors.

### §B — Multiset.iter() yields no elements

* §B.1 `union(&other).count(&k)` returns self.count(k) (iter body
  never runs)
* §B.2 `intersection(&other).count(&k)` returns 0 (iter body never
  runs)
* §B.3 `sum(&other).count(&k)` returns self.count(k)
* §B.4 `difference(&other).count(&k)` returns 0
* §B.5 direct `iter().next()` loop visits 0 elements

Root: MultisetIter wraps Map.iter(), and the wrapper-type iterator
inherits the `Map` runtime kind.  CallM("next", iter) dispatches
against Map's method table, where `next` is absent.  Same fix path
as slice §A / §D close-out.

## 5. Action items

### Landed in this branch

1. Unit-test surface — 30 tests across 10 sections:
   * Section 1 — Construction (new / with_capacity)
   * Section 2 — Insertion (insert / insert_n positive / zero / negative / count / contains absent)
   * Section 3 — Removal (remove / remove_n / remove_all + saturation)
   * Section 4 — clear
   * Section 5 — Cardinality bookkeeping (inserts + removes)
   * Section 6 — Union (per-element max, with-empty)
   * Section 7 — Intersection (per-element min, with-empty)
   * Section 8 — Sum (per-element addition)
   * Section 9 — Difference (per-element saturating subtraction)
   * Section 10 — Subset / superset / empty-is-subset
2. Property-test surface — 10 algebraic laws over (ℕ, +, max, min):
   cardinality coherence; insert_n(0) no-op; insert/remove inverse;
   remove_all idempotent; remove_n saturation; clear identity;
   distinct_len vs cardinality independence; count-zero iff
   !contains; insert ≡ insert_n(_, 1); cardinality monotone.
3. Integration tests — 5 cross-type scenarios:
   * Word frequency from List<T> stream
   * Per-key drain via remove_all
   * Alternating insert/remove balances
   * No-op operations preserve state
   * Clear restores to fresh-new state
4. Regression suite — 2 @ignore'd defect pins for §A + 4 PASS-GUARDs
   for the working surface (new is empty, insert/remove roundtrip,
   two-counter invariant, clear restoration).

### Deferred

1. **Close §A** — fold Multiset.from / from_counts into the cross-
   module constructor name-table fix path.  Same architectural class
   as List / Map / Set / Deque constructors.
2. Property-test sweep over algebraic-operations laws once the
   `from(...)` constructor is reachable from user code:
   * Union absorption — `A ∪ A = A`
   * Union commutativity — `A ∪ B = B ∪ A`
   * Intersection absorption — `A ∩ A = A`
   * Intersection commutativity — `A ∩ B = B ∩ A`
   * Sum commutativity — `A ⊎ B = B ⊎ A`
   * Difference self-annihilation — `A − A = ∅`
   * Subset-superset duality — `A ⊆ B ⇔ B ⊇ A`
   * Subset reflexivity — `A ⊆ A`
   * Empty is universal subset — `∅ ⊆ A` for all A

## 6. Status of dependents

When the §A constructor gap closes, `Multiset.from([...])` /
`Multiset.from_counts([...])` builder paths immediately unlock
ergonomic frequency-counter idioms across:

* `core/text/word_frequency.vr` (if introduced)
* statistical aggregation pipelines
* test fixtures that need quick multiset construction
