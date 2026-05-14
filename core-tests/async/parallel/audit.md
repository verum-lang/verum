# `core/async/parallel.vr` — audit

Pure-data collection-processing primitives.  Today every function is a
sequential fallback; the test contract is the *result* (the in →
out relation), which the parallel rewrite must preserve.

## Public API surface

| Name | Shape | Status under interpreter |
|---|---|---|
| `parallel_map<T,U>(List<T>, Int, fn(T)->U) -> List<U>` | length-preserving lift | green (6 unit, 2 property) |
| `parallel_filter_map<T,U>(List<T>, Int, fn(T)->Maybe<U>) -> List<U>` | filtering lift | green (5 unit, 1 property) |
| `parallel_for_each<T>(List<T>, Int, fn(T)->())` | side-effect sink | green (3 unit) |
| `parallel_reduce<T>(List<T>, Int, fn(T,T)->T) -> Maybe<T>` | fold₁ | green (5 unit, 2 property) |
| `parallel_scan_exclusive(List<Int>, Int, fn(Int,Int)->Int) -> List<Int>` | Blelloch exclusive prefix scan | green (7 unit, 2 property) |

Unit tests cover boundary cases (empty input, singleton, size-2, the
canonical [1,2,3,4] → [0,1,3,6] example) and the worker-count
hint-only contract (negative / zero / mismatched worker_count must
not change the result).

## Cross-stdlib usage

| Site | Risk |
|---|---|
| Used inside `core/text/` parallel-walk routines and `core/collections/` algorithms when present | None observed |

## Crate-side hardcodes

No special-case opcode / WKT reservation observed.  Functions go
through the normal user-function dispatch path.

## Language-implementation gaps

1. **AOT global crash — SIGABRT in compiler.phase.generate_native.**
   Reproduces on every test under `--aot`, not specific to parallel
   (base/maybe `test_none_construction` crashes identically).
   Backtrace shows `__pthread_cond_wait` then `llvm::SmallVectorBase::grow_pod`.
   Tracked separately under task #10.  Interpreter tier is fully green.

## Action items landed in this branch

* Created `core-tests/async/parallel/{unit_test.vr,property_test.vr,integration_test.vr,audit.md}`.
* 38 tests under interpreter — all green.
* Pinned worker_count-invariance properties (varying the worker hint
  must not change the result), the textbook Blelloch [1,2,3,4]→[0,1,3,6]
  identity, and the reference-equivalent exclusive prefix-scan-vs-
  reference identity over six representative input sizes.

## Action items deferred

* AOT validation pending task #10 (LLVM SmallVector hang during
  generate_native, global compiler defect).
* Once `core.async.executor` supports work-stealing parallelism, the
  five functions need new tests confirming non-trivial speedup and
  the absence of result divergence vs sequential fallback.
