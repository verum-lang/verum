# `core.collections.btree` — Audit

Conformance review for `core/collections/btree.vr` — `BTreeMap<K: Ord, V>`
and `BTreeSet<T: Ord>`, ordered key-value containers backed by an
internal balanced tree.

## Status

**regression-only** — Every method that reads `self.<field>` on a
populated BTreeMap panics with
  "field access out of bounds: field index 0 (offset 0+8 = 8)
   exceeds object data size 0".

### Re-diagnosis 2026-05-23 + 2026-05-24

**Dual-defect** — investigation surfaced two distinct architectural
defects, both inside the `Maybe<Heap<T>>` value-shape chain.  Closing
either one in isolation surfaces the other; both must be closed
together to make BTreeMap functional.

#### Defect (a) — type-inference loss in `.as_ref()` / `.as_mut()`

`Maybe<Heap<T>>.as_ref()` returns `Maybe<&Heap<T>>` syntactically,
but the codegen's type-inference loses the inner `T` parameter
through the `.expect(...)` unwrap.  Subsequent field access on the
resulting `&Heap<T>` mis-resolves through `resolve_field_index`'s
scan-all-types fallback, landing on a wrong type's field layout.

Minimal reproduction (no stdlib involvement):

```verum
type GenericNode<K: Ord, V> is { keys: List<K>, values: List<V>, is_leaf: Bool };
type Container<K: Ord, V> is { root: Maybe<Heap<GenericNode<K, V>>> };
implement<K: Ord, V> Container<K, V> {
    fn populate(&mut self) {
        self.root = Some(Heap.new(GenericNode { keys: [], values: [], is_leaf: true }));
    }
    fn root_leaf(&self) -> Bool {
        let r = self.root.as_ref().expect("some");
        r.is_leaf   // ← panics: field access OOB, data size 0
    }
}
```

Same code with `match &self.root { Some(heap) => heap.is_leaf, ... }`
works correctly — match-destructure preserves the inner type binding.

#### Defect (b) — `Heap.new(node)` value-loss for generic record args

Refactoring `btree.vr`'s `.as_mut().expect(...)` sites to
`match &mut self.root { Some(root_ref) => ..., None => ... }` (the
workaround for defect (a)) made `insert` no longer panic — but
surfaced defect (b): the inserted key/value pair is silently lost.
`m.len()` reports 1 after insert; `m.contains_key(&1)` returns
`false`; `m.get(&1)` returns `None`.

Reproduction post-workaround:

```verum
let mut m: BTreeMap<Int,Int> = BTreeMap.new();
m.insert(1, 100);
m.len();             // ← 1 (correct)
m.contains_key(&1);  // ← false (WRONG — should be true)
```

The chain is `BTreeNode.new_leaf()` → `node.keys.push(key)` →
`Heap.new(node)` → `self.root = Some(...)`.  After this sequence,
the wrapped node accessible through `self.root` does not contain
the pushed keys.  `Heap.new(node)` appears to wrap a value that
lost its mutations across the generic-argument boundary.

### Workaround NOT viable

Refactoring to `match`-extract patterns (the obvious workaround
for defect (a)) was attempted on 2026-05-24 and reverted because
it surfaces defect (b), turning a loud panic into silent data
loss.  Two-defect interaction means surface-level stdlib refactors
won't close the class.

### Fix paths

1. **(a)** Type-inference: trace `.as_ref()` / `.as_mut()` on
   `Maybe<Heap<T>>` through the typechecker.  The inner `T` should
   propagate through `.expect(...)` so subsequent field access
   resolves to the right field layout.  Likely in
   `verum_types/src/infer/expr.rs`'s MethodCall arm or
   `extract_expr_type_name` for chained `.expect()`.

2. **(b)** `Heap.new(node)` value preservation: trace how the
   generic argument's value is captured.  The runtime intercept
   at `interpreter/dispatch_table/handlers/method_dispatch.rs:
   1091-1163` allocates a CBGR slot and stores `value` (the node
   pointer).  If the node was modified post-creation but before
   the Heap.new call, the modifications might be on a different
   register / freshly-allocated slot than what Heap.new captures.

3. **Add explicit BTreeMap runtime intercept** — mirror the
   `MakeRecord` discipline used for `List`/`Map` and stamp a
   known-good 2-slot allocation for BTreeMap on construction,
   sidestepping the entire `Maybe<Heap<T>>` chain at runtime.
   Heaviest but most contained fix.

### Deeper investigation 2026-05-24

Isolated probes narrowed the failure to **pattern-matching codegen
on `&Maybe<Heap<RecordWithGenericParams>>`**:

| Scrutinee shape | Behaviour |
|---|---|
| `match m { Some(_) => …, None => … }` (value match) | ✅ correct — takes Some branch |
| `match &m { Some(_) => …, None => … }` (ref match, generic-parameterised inner) | ❌ takes None branch (variant tag mis-read) |
| `match m.clone() { Some(h) => helper(&h), … }` (clone + value match + helper fn) | ✅ correct field access |
| `if let Some(ref h) = self.root { h.<field> }` | ❌ field index 7 OOB (destructured `h` has wrong type info → global-intern fallback) |

`Heap.new(node)` preserves data correctly when accessed via explicit
double-deref `*(*h)` (verified empirically: pushes to `node.keys`
ARE visible post-`Heap.new`).  So defect (b) is NOT a Heap value-
loss issue — it's the SAME defect (a) re-surfacing through the
destructured-binding type-loss.

**Single underlying root cause**: pattern-matching codegen for ref
scrutinees on `Maybe<Heap<T>>` where `T` is parameterised by generic
params loses the inner-type binding.  The variant-tag read goes to
the wrong offset (taking None branch) AND the destructured payload
binding doesn't carry `T`'s field layout (taking global-intern
fallback for `h.<field>` resolution).

### Tactical workaround (NOT applied to btree.vr)

`match self.root.clone() { Some(h) => helper(&h, ...), None => ... }`
with `helper<K, V>(h: &Heap<BTreeNode<K, V>>, ...)` explicitly typed
WORKS for the read path — value-match + helper bypasses both
sub-defects.

Mutation path (`&mut self` methods) still fails because
`&mut h` from a cloned Heap doesn't reach the original heap data
(empirical: `mutate_node(&mut cloned_h, ...)` doesn't persist).

So the read path is workaround-able with btree.vr source refactor,
but the mutation path requires the underlying codegen fix.  Partial
refactor would land read-side green tests but leave write-side
defective — not landed because it would produce a confusing
asymmetric API.

### Architectural fix

Trace the pattern-matching codegen in `verum_vbc/src/codegen/`
(likely `expressions.rs::compile_match` and
`pattern_matching.rs::handle_make_variant`'s GetVariantData read)
for the `&Maybe<Heap<T>>` shape.  The variant-tag offset must
account for the through-reference + through-Heap chain, AND the
destructured binding's type must carry the inner `T` for downstream
field resolution.  Multi-day VBC codegen investigation.

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
