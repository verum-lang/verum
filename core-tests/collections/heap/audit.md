# `core.collections.heap` — Audit

Conformance review for `core/collections/heap.vr` (BinaryHeap<T: Ord> —
max-heap priority queue backed by List<T>).

## Status

**partial** — Unit / property / integration coverage spans construction
(new, with_capacity, from_list), basic priority-queue ops (push, pop,
peek, clear), and full-drain sorted output (into_sorted_list). Two
architectural fixes landed in this branch:

1. **Cross-module name table for non-intercepted static constructors**
   — `BinaryHeap.with_capacity(n)` / `BinaryHeap.from_list(xs)` now
   resolve through the canonical user-side function table (parallel
   agent's earlier task close-out).
2. **Array-dispatch polarity defect** — closed via `dispatch_primitive_method`
   gating on the positive `header.type_id.is_array_dispatchable()`
   predicate instead of the broken negative `is_value_array && type_id
   < 256` pair. See regression §C for full root-cause analysis.

## 1. Cross-stdlib usage

Heap is the canonical priority queue. Used at:

| Site | Shape | Notes |
|---|---|---|
| `core/async/scheduler.vr` (priority-aware scheduling paths) | `BinaryHeap<TaskWithPriority>` | priority queue for ready tasks |
| `core/cog/build_graph.vr` (path varies) | `BinaryHeap<(JobId, Cost)>` | longest-path / critical-path scheduling |
| `core/math/numeric/sort.vr` (path varies) | `BinaryHeap<T>` | heap-sort building block |

## 2. Crate-side hardcodes

BinaryHeap has **no direct** runtime intercept. Every method dispatches
through `self.data.<method>` on the inner `List<T>`, which IS runtime-
intercepted. The wrapper-record indirection is safe because:

* `BinaryHeap { data: List.new() }` field initialisation places the inner
  List pointer at slot 0 of the heap record.
* `self.data` codegen reads slot 0 and yields the inner List object.
* Subsequent method dispatch on the List object hits the List runtime
  intercepts.

## 3. Language-implementation gaps

| Gap | Impact | Fundamental fix |
|---|---|---|
| Array-dispatch polarity defect (negative gate `type_id != X && type_id < 256` mis-routed every stdlib type whose `alloc_user_type_id` cursor crossed 256 into the array intercepts) — **CLOSED** | `BinaryHeap.new().len()` returned 1 (the heap object's slot count from `header.size / sizeof::<Value>()`) instead of 0. Affected every stdlib record whose TypeId landed in [260, 512) — the gap between meta-system and semantic ranges. | Polarity invert: gate on `header.type_id.is_array_dispatchable()` — single source of truth (LIST=512, ARRAY=518, BYTE_LIST=527 only). Mirrors `dispatch_array_method`'s identical invariant. |
| Heap depends transitively on the List intercept correctness — every List defect propagates here | Indirect. Audited via List audit §3 line items; heap doesn't add new defects. | Close the List defects (List.set / List.get_or / runtime memory-layout drift all closed earlier in this branch). |

## 4. Defect inventory

* `BinaryHeap.with_capacity` / `BinaryHeap.from_list` — CLOSED (active guardrails in regression §A).
* Array-dispatch polarity (`h.len()` returned 1 for empty heap) — CLOSED (active guardrails in regression §C: `regression_heap_len_zero_after_new` / `regression_heap_is_empty_after_new`).
* `regression_heap_wrapper_field_resolves_to_inner_list` — active guardrail (§B).

## 5. Action items

All known heap defects are closed. Further work would target audit of
transitive List defects as List's audit closes them.
