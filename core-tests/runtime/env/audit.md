# `runtime/env` audit

Module: `core/runtime/env.vr` (~1016 LOC) — runtime environment
pillars: memory context, capabilities, supervision, concurrency,
and execution tier selection.

Tests: 26 unit tests covering pure-data subset: ExecutionTier
4-variant + .overhead_ns CBGR cost contract (15/8/3/0 ns) +
.requires_generation + .requires_epoch + EnvIsolationLevel
3-variant + CpuAffinity 3-variant + EnvTaskId record + .main.

Stateful surfaces (MemoryContext, GenTracker, SharedRegistry,
ConcurrencyContext, RecoveryContext) are intrinsic-backed and
gated by VBC runtime support — covered at vcs/specs/L2-standard/
runtime/.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.async.spawn` | reads ExecutionTier from ambient runtime env. |
| `core.runtime.cbgr` | per-task GenTracker + ExecutionTier. |
| `core.runtime.supervisor` | drives RuntimeRestartPolicy decisions. |
| `verum_vbc::interpreter` | reads tier to dispatch the CBGR check path. |

## 2. Crate-side hardcodes

* ExecutionTier `overhead_ns` values (15/8/3/0) MUST agree with
  `verum_vbc::cbgr::overhead_target_ns`; drift breaks the
  production_targets benchmark contract.
* The CBGR check overhead target ("< 15ns") in CLAUDE.md is a
  performance contract pinned by Tier0_Full.overhead_ns().

## 3. Language-implementation gaps

### §3.1 Property test on tier overhead monotonicity

∀t1, t2: ExecutionTier. tier_strictness(t1) > tier_strictness(t2)
⟹ t1.overhead_ns() ≥ t2.overhead_ns(). Pinned in this branch's
test_execution_tier_overhead_monotone_decreasing.

### §3.2 Add Display/Debug for ExecutionTier

Useful for runtime trace messages. Currently emitted as raw
variant name via debug-only print.

**Effort:** small (~15 min).

### §3.3 Test for MemoryContext.fork SendBoundary semantics

`fork()` resets gen_tracker and marks send_boundary = true.
Requires non-static MemoryContext ctor that doesn't depend on
allocator pointer; gated by `core.mem.alloc` interp support.

### §3.4 SharedRegistry.global() singleton test

Returns &Self; assert pointer-equal on repeated calls. Gated by
intrinsic backing for the global ptr.

## Action items landed in this branch

* `core-tests/runtime/env/unit_test.vr` — 26 unit tests over
  ExecutionTier + EnvIsolationLevel + CpuAffinity + EnvTaskId.
* `core-tests/runtime/env/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add Display/Debug for ExecutionTier | `core/runtime/env.vr` + tests | 15 min |
| Property test on tier overhead monotonicity | this folder | 30 min |
| MemoryContext.fork SendBoundary test | this folder | gated on alloc interp |
| Sister tests for `core.runtime.{spawn,supervisor,recovery,thread,pool,task_queue}` | sister folders | 1 week total |
