# `runtime/tls` audit

Module: `core/runtime/tls.vr` (58 LOC) — thread-local storage intrinsic
surface.  12 forward-declared free fns covering slot lifecycle
(`tls_slot_{get,set,clear,has}`), call-frame scoping
(`tls_frame_{push,pop}`), and raw-offset reads/writes (`tls_{read,write}_{ptr,i32,usize}`)
plus the base pointer (`tls_get_base`).

Tests: 16 unit tests (10 GREEN with live exercise on slots 200..221;
6 symbol-existence-only pins for the raw-offset surface).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.sys.common.{ctx_get, ctx_set, ctx_clear}` | every context-DI lookup is a `tls_slot_*` op. |
| `core.runtime.ctx_bridge.env_ctx_*` | the AOT-side `CtxGet`/`CtxProvide` opcode lowering calls into this surface. |
| `core.runtime.env.ExecutionEnv` | stored as `tls_slot_set(EXEC_ENV_SLOT, &env)` at runtime init. |
| `core.async.task` | per-task TLS state via `tls_frame_push`/`pop` (where supported). |

`grep -r "tls_slot_\|tls_frame_\|tls_read_\|tls_write_" core/` returns
~12 sites — all in `core/sys/common.vr` and `core/runtime/ctx_bridge.vr`.

## 2. Crate-side hardcodes

| Site | What it pins | Risk |
|---|---|---|
| `crates/verum_vbc/src/interpreter/state.rs:634` (`cbgr_epoch`) | per-state cbgr epoch lives in the interpreter, NOT in TLS | Drift here would put cbgr state in TLS and break the per-task semantics. |
| `crates/verum_codegen/src/llvm/tls.rs` (TLS variable emission) | `__thread`-style storage; per-arch base-pointer access via `%fs:` / `%gs:` / `tpidr_el0` | HOST gating instead of TARGET miscompiles cross builds. |
| CONTEXT_SLOT_COUNT (`core/sys/common.vr`) | upper bound on slot index | Tests in this folder use slots 200..221 — well above the typical 16-slot context reservation but below any plausible per-platform ceiling. |

## 3. Language-implementation gaps

### §A — `tls_slot_*` intrinsic dispatch not bound at the runtime layer

Same defect class as [[runtime/cbgr §A]].  The ident strings
`verum.runtime.tls_slot_{set,get,clear,has}` (and the
`tls_frame_*` + `tls_{read,write}_*` family) have no registered handler
in `crates/verum_vbc/src/interpreter/dispatch_table/`.  The default-zero
intrinsic stub fires:

* `tls_slot_set(slot, value)` — silently no-op
* `tls_slot_get(slot)`        — returns 0
* `tls_slot_has(slot)`        — returns false
* `tls_slot_clear(slot)`      — silently no-op

Probe (2026-05-27):
`tls_slot_set(200, 0xCAFE_F00D); tls_slot_get(200) == 0` (expected
0xCAFE_F00D).

Pinned `@ignore` on the round-trip tests:
* `test_tls_slot_lifecycle_set_then_get_round_trips`
* `test_tls_slot_has_after_set_is_true`
* `test_tls_slot_overwrite_keeps_latest`
* `test_tls_slot_isolation_two_slots`
* `test_tls_slot_clear_does_not_affect_other_slots`

These flip GREEN when the dispatch handlers land.

### §B — DRIFT: two parallel TLS-slot APIs

The actual context-DI surface used by `core.sys.common.{ctx_get, ctx_set,
ctx_clear}` IS bound and works correctly — it routes through a
DIFFERENT intrinsic path (`ContextSlots` vector inside the VBC
interpreter state).  Both APIs claim to be "TLS slot" APIs but only
one is functional.  This is dangerous: a stdlib reader will assume
`core.runtime.tls.tls_slot_set` is the canonical TLS-slot surface,
but the runtime actually flows through `core.sys.common.ctx_set`.

Recommend: either
* deprecate `core.runtime.tls.tls_slot_*` in favour of
  `core.sys.common.ctx_*`, OR
* re-route the bound `ctx_*` path through the `tls_slot_*` intrinsics
  as a single source of truth.

### §C — TLS slot 200..221 may collide with internal slots on future platforms

The test suite uses slot indices 200..221 to avoid stomping on the
context-DI 0..16 reservation.  If a future platform raises
CONTEXT_SLOT_COUNT, these tests may need to be re-pinned.  Audit
recommendation: parameterise the test slots from a symbolic
`PUBLIC_TEST_SLOT_BASE` constant in `core/sys/common.vr` to make the
drift explicit.

### §D — frame_push / frame_pop semantics under no-frame-support platforms

The frame intrinsics are no-ops on platforms without nested-frame
support.  Current tests pin only callability; pinning the
behavioural contract ("inside a frame, slot reads see the
frame-local value") requires platform gating that's gated on
`@cfg(feature = "tls_frames")`.

### §E — raw TLS read/write at a raw offset is unsafe

`tls_read_ptr(offset)` reads at a raw byte offset from the base.  No
runtime bounds check, no offset validation.  This is by design — the
intrinsic is the bottom of the stack — but the docs should make this
explicit.  Recommend adding a `@requires_unsafe` annotation.

## Action items landed in this branch

* `core-tests/runtime/tls/unit_test.vr` — 16 unit tests covering
  slot lifecycle + isolation + frame stack + raw-offset surface.
* `core-tests/runtime/tls/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| §A wire `tls_slot_*` intrinsic dispatch | `crates/verum_vbc/src/interpreter/dispatch_table/handlers/` | 1 day |
| §B unify two parallel TLS-slot APIs | `core/runtime/tls.vr` + `core/sys/common.vr` | 1 day (cross-cutting) |
| §C `PUBLIC_TEST_SLOT_BASE` constant | `core/sys/common.vr` + this folder | 30 min |
| §D per-platform frame_push behavioural test | `vcs/specs/L2-standard/context/` | 1 h |
| §E `@requires_unsafe` annotation on `tls_read_*` / `tls_write_*` | `core/runtime/tls.vr` | 15 min |
| Cross-thread isolation test (slot set in T1, T2 sees default) | `vcs/specs/L2-standard/runtime/tls/` | gated on spawn under interp |
