# `core.collections.map` — Audit

Conformance review for `core/collections/map.vr` (Map<K: Hash + Eq, V> —
Robin Hood hash map).

## Status

**partial** — Unit / property / integration coverage spans the full
runtime-intercepted API (new, insert, contains_key, get → Maybe<V>,
remove, clear, len). Two fundamental fixes landed in this branch:

* **Layout-drift CLOSED**: stdlib type decl reordered to
  `{ len, cap, entries, tombstones }` to match the VBC runtime
  intercept slot allocation `[len@0, cap@1, entries_ptr@2,
  tombstones@3]`. Runtime allocation extended from 3 slots to 4 slots
  so the stdlib `tombstones`-using resize path agrees with the
  intercept-allocated heap object.
* **Map.get(&key) CLOSED**: auto-deref CBGR-ref key in the intercept's
  `value_hash` / `value_eq` path. Mirror fix applied to Map.contains_key
  / Map.remove / Set.contains / Set.remove. Same defect class as the
  earlier List.contains needle deref fix.

The Map.get fix unblocks the union_find + toposort regression cascade
that pinned this defect cross-collection.

## 1. Cross-stdlib usage

Map is the most-touched stdlib type after List. Hot call sites:

| Site | Shape | Notes |
|---|---|---|
| `core/collections/set.vr` | `Set<T> is { inner: Map<T, ()> }` | Set's backing store. |
| `core/collections/toposort.vr` | `Map<Node, Visited>` | DFS state tracking. |
| `core/cog/dep_graph.vr` (path varies) | `Map<CrateId, List<CrateId>>` | adjacency lists. |
| `core/proof/...` (path varies) | `Map<HypothesisId, Hypothesis>` | proof-state binding. |
| `core/database/...` | `Map<ColumnId, Column>` | schema metadata. |

## 2. Crate-side hardcodes

| Path | Line(s) | What it does |
|---|---|---|
| `crates/verum_vbc/src/interpreter/dispatch_table/handlers/method_dispatch.rs` | 1060-1085 | `Map.new()` runtime intercept — allocates `[count, capacity, entries_ptr]` with TypeId::MAP. |
| same | 4337-4990 | Map instance-method intercepts: `len`, `iter`, `insert`, `contains_key`, `remove`, `clear`, `get`. |
| same | 4634+ (this branch) | Cap=0 bootstrap guard on `insert` — required when the constructor ran through the stdlib body (cap=0) instead of the static-constructor intercept; resize math hits divide-by-zero otherwise. Same guard mirrored at Set.insert. |

## 3. Language-implementation gaps

| Gap | Impact | Fundamental fix |
|---|---|---|
| Stdlib type decl `Map<K, V> is { entries, len, cap, tombstones }` (4 fields) vs runtime intercept `[count, capacity, entries_ptr]` (3 slots — no tombstones). | Every non-intercepted stdlib method reading `self.entries` / `self.len` / `self.cap` / `self.tombstones` misroutes. The tombstones field has no slot in the runtime layout at all; any code path that reads/writes it is structurally broken. | Reconcile: either (a) extend runtime intercept layout to 4 slots including tombstones, (b) collapse the stdlib type decl to `{ count, capacity, entries_ptr }` raw, or (c) add Rust-side intercepts for every method that reads `self.<field>` on Map. |
| `Map.from(pairs)` / `Map.with_capacity(n)` cross-module UndefinedFunction | Pinned in regression §A. Same defect class as List.from / Set.from / Deque.with_capacity. | Same fix path as #24/#25/#26 close-out. |
| `Map.get → Maybe<V>` dispatch (cross-cited in `union_find` / `toposort` audits as the defect that gates contains-key-plus-get patterns). | Pinned in regression §B. Some call shapes return inner value, some return Maybe<&V>; pattern-matching cannot destructure either deterministically. | Re-implement the get intercept to always return a freshly constructed `Maybe<V>` (variant-tagged) regardless of receiver shape. |
| Map.new through stdlib body returns cap=0 — divide-by-zero in resize math. | CLOSED in this branch via the bootstrap guard described above. | — |

## 4. Defect inventory

* `Map.from` / `Map.with_capacity` cross-module UndefinedFunction (§A, ignored).
* `Map.get` → Maybe<V> destructure mismatch (§B, ignored).
* `Map.iter` via stdlib-body fall-through (§C, ignored).
* Cap=0 bootstrap (CLOSED in this branch).

## 5. Action items

1. Close the cross-module constructor name-table gap for `Map.from` and
   `Map.with_capacity` (same item as List/Set/Deque audits).
2. Re-implement `Map.get` to always return `Maybe<V>` with the canonical
   variant shape — unblocks union_find / toposort / many user-side
   call sites.
3. Reconcile the stdlib `{ entries, len, cap, tombstones }` decl with
   the runtime `[count, capacity, entries_ptr]` layout — long-term
   fundamental cleanup; either drop tombstones or add a 4th slot.
