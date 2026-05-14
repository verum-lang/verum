# `core.collections.set` — Audit

Conformance review for `core/collections/set.vr` (Set<T: Hash + Eq> —
hash-set backed by Map<T, ()> at the stdlib level, intercepted directly
as `[count, capacity, entries_ptr]` with TypeId::SET at runtime).

## Status

**partial** — Unit / property / integration coverage targets the API
surface with explicit runtime intercepts (new, insert, contains, remove,
clear, union, intersection, len, is_empty). Methods that go through the
stdlib body's `self.inner.<method>` are pinned in regression_test.vr §A
because the wrapper-record stdlib decl `{ inner: Map }` doesn't match
the direct `[count, capacity, entries_ptr]` runtime allocation shape.

## 1. Cross-stdlib usage

`Set<T>` is the canonical deduplication / membership-tracking primitive,
used in:

| Site | Shape | Notes |
|---|---|---|
| `core/collections/multiset.vr` | Set ⊕ count map | multiset backbone |
| `core/collections/toposort.vr` | `Set<Node>` for "visited" tracking | DFS-based topological sort visited set |
| `core/proof/...` (path varies) | `Set<HypothesisId>` | Used dependent-hypothesis tracking |
| `core/cog/dep_graph.vr` (path varies) | `Set<CrateId>` | reachability closure |

## 2. Crate-side hardcodes

| Path | Line(s) | What it does |
|---|---|---|
| `crates/verum_vbc/src/interpreter/dispatch_table/handlers/method_dispatch.rs` | 1060-1085 | `Set.new()` runtime intercept — allocates `[count, capacity, entries_ptr]` with TypeId::SET. |
| same | 4336-5300 | Set instance-method intercepts: `len`, `iter`, `insert`, `contains`, `remove`, `clear`, `is_empty`, `union`, `intersection`. |

## 3. Language-implementation gaps

| Gap | Impact | Fundamental fix |
|---|---|---|
| Stdlib type decl `Set<T> is { inner: Map<T, ()> }` is a wrapper record but the runtime intercept allocates `[count, capacity, entries_ptr]` directly with TypeId::SET. | Methods that read `self.inner` (every non-intercepted stdlib method — to_list, is_subset, is_superset, is_disjoint, difference, symmetric_difference, retain, find, any, all on Set) get a wrong-shape receiver. | Reconcile by either (a) collapse stdlib type to `{ count, capacity, entries_ptr }` raw; (b) extend the runtime intercept to allocate a 1-slot wrapper containing a Map; (c) add Rust-side intercepts for every Set method that touches `self.inner`. Option (a) is structurally cleanest because it removes the wrapper-level indirection. |
| `Set.from(values)` cross-module UndefinedFunction | Pinned in regression §A. Same cross-module name-table gap as List.from / List.of / List.from_elem. | Same fix path as #24/#25/#26 close-out. |

## 4. Defect inventory

* `to_list`, `is_subset`, `difference` — three exemplars of the wrapper-
  shape drift; ignored, kept as guardrails until §A closes.
* `Set.from([...])` — cross-module name-table gap; ignored.

## 5. Action items

1. **Close the wrapper-shape drift** (§3 row 1) using option (a) — change
   stdlib decl. This makes every stdlib `&self` / `&mut self` method
   automatically route to the right slot.
2. **Close the cross-module constructor name-table gap** for `Set.from`.
