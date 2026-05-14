# `core.collections.heap` — Audit

Conformance review for `core/collections/heap.vr` (BinaryHeap<T: Ord> —
max-heap priority queue backed by List<T>).

## Status

**partial** — Unit / property / integration coverage targets construction
(new), basic priority-queue ops (push, pop, peek, clear), and full-drain
sorted output (into_sorted_list). `BinaryHeap.with_capacity(n)` and
`BinaryHeap.from_list(xs)` constructors are pinned in regression §A
because they share the cross-module function-name resolution defect class
with List/Deque/Set non-intercepted constructors.

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
| `BinaryHeap.with_capacity(n)` / `BinaryHeap.from_list(xs)` cross-module UndefinedFunction | Pinned in regression §A. | Same fix path as #24/#25/#26 close-out. |
| Heap depends transitively on the List intercept correctness — every List defect propagates here | Indirect. Audited via List audit §3 line items; heap doesn't add new defects. | Close the List defects (List.set / List.get_or already closed in this branch; runtime memory-layout drift open). |

## 4. Defect inventory

* `BinaryHeap.with_capacity` — cross-module UndefinedFunction (ignored).
* `BinaryHeap.from_list` — cross-module UndefinedFunction (ignored).
* `regression_heap_wrapper_field_resolves_to_inner_list` — active
  guardrail that fails immediately if the wrapper-record indirection
  ever breaks.

## 5. Action items

1. Close cross-module constructor name-table gap (audit.md §3, same
   item as List/Deque/Set audits).
2. Audit heap for transitive List-defect impact once the List runtime
   memory-layout drift closes — heap's drain/extract_if/retain shapes
   should re-enable test coverage at that point.
