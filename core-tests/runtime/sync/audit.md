# `runtime/sync` audit

Module: `core/runtime/sync.vr` (10 LOC) — pure intrinsic surface; single
function `spin_loop_hint()` emitting the CPU pause / yield hint.

Tests: 3 unit tests covering callability + zero-iteration safety + unit
return type.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.sync.spinlock` / `core.sync.atomic` (spin variants) | one call per fail-cas iteration to back off pressure on the cache line. |
| `core.concurrency.session` (lock-free queues) | spin between work-stealing attempts. |
| `core.async.task_queue` (work-stealer) | spin in the steal-fast-path loop before falling back to mutex+condvar. |

## 2. Crate-side hardcodes

| Site | What it pins | Risk |
|---|---|---|
| `crates/verum_codegen/src/llvm/instruction.rs` (`pause` / `yield` lowering) | Per-target instruction selection | Drift drops to a generic LLVM `llvm.donothing` intrinsic, silently negating the spin-wait optimization. |

`grep` for the bound intrinsic ident:

```
grep -rn "verum.runtime.spin_loop_hint" crates/
```

Currently empty — see §A.

## 3. Language-implementation gaps

### §A — spin_loop_hint intrinsic not dispatch-bound

Same defect class as [[runtime/cbgr §A]].  The ident
`@intrinsic("verum.runtime.spin_loop_hint")` at
`core/runtime/sync.vr:9` has no registered handler in `crates/verum_vbc`
or `crates/verum_codegen`.  Under --interp the call appears to be a
no-op (the test suite passes), so the user-side effect is correct by
accident: a hint with no implementation IS a no-op.  Under --aot the
emitted instruction sequence falls through to whatever LLVM does for
unbound intrinsics, which is likely an indirect function call with
ABI overhead — defeating the entire purpose of the hint.

Fix surface (same plan as cbgr §A): register the ident in both VBC
and codegen.  At codegen lower to per-target `pause`/`yield`/`wfe`.

## Action items landed in this branch

* `core-tests/runtime/sync/unit_test.vr` — 3 unit tests (callable +
  zero-iteration + return-type).
* `core-tests/runtime/sync/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| §A wire `spin_loop_hint` in VBC + codegen | `crates/` | 30 min once cbgr-§A scaffold lands |
| AOT instruction-selection drift pin in `verum_codegen` tests | crate-side | 15 min |
| Live spin-wait benchmark vs naked-loop baseline | `vcs/benchmarks/micro/` | 1 h |
