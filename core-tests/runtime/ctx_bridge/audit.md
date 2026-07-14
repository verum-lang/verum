# `runtime/ctx_bridge` audit

Module: `core/runtime/ctx_bridge.vr` (122 LOC) — bridge functions
between AOT `CtxGet`/`CtxProvide`/`CtxEnd` opcodes and the bound
`sys.common.ctx_*` TLS-slot path.

Tests: 19 unit tests covering out-of-range guards, round-trip, slot
isolation, overflow guards on `env_install_parent_contexts`, and
`env_active_slot_count` self-consistency.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `crates/verum_codegen/src/llvm/instruction.rs` (`CtxGet` lowering) | AOT-emitted `CtxGet(slot)` lowers to `call @env_ctx_get(i64 slot) -> i64` (`core/runtime/mod.vr:643`). |
| `crates/verum_codegen/src/llvm/instruction.rs` (`CtxProvide` lowering) | calls `env_ctx_set`. |
| `crates/verum_codegen/src/llvm/instruction.rs` (`CtxEnd` lowering) | calls `env_ctx_end`. |
| `core.async.spawn` (parent-context fork) | calls `env_install_parent_contexts(slots_ptr, count)` after thread creation. |

## 2. Crate-side hardcodes

| Site | What it pins | Risk |
|---|---|---|
| `crates/verum_codegen/src/llvm/instruction.rs` (CtxGet/Provide/End opcodes) | ABI: `(i64 slot) -> i64` for Get; `(i64 slot, i64 value)` for Set/End | ABI drift between codegen and ctx_bridge.vr surfaces as silent slot-id-mismatch. |
| `CONTEXT_SLOT_COUNT` (`core/sys/common.vr`) | upper-bound check at every guard | Drift here either OVER-permits (allows out-of-range slots through) or UNDER-permits (rejects valid slots). |
| 16-byte stride (`ctx_bridge.vr:104`) | per-entry layout in the slots buffer (`(slot_id: i64, value: i64)`) | Caller-emitted buffer must match; drift between spawn trampoline and this layout silently corrupts the per-thread context. |

## 3. Language-implementation gaps

### §A — `env_ctx_set` / `env_ctx_get` round-trip not bound under --interp

Surfaced 2026-05-27.  Same defect class as [[runtime/tls §A]]:

* `env_ctx_set(8, 0xBEEF); env_ctx_get(8)` returns 0 (expected 0xBEEF).
* The source delegates to `sys.common.ctx_set` / `ctx_get` via an
  Int→`&unsafe Byte`→Int cast chain — either the underlying
  `ctx_set` is a stub OR the cast boundary drops bits.

The codegen-emitted `CtxGet` / `CtxProvide` opcodes (NOT these
user-callable bridge fns) DO work — those are what compiled
`provide ... in { ... }` lowers to.  The drift is the same as
[[runtime/tls §B]]: two parallel paths claim to be "the TLS bridge",
only one is bound.

Pinned `@ignore` on round-trip tests:
* `test_env_ctx_set_get_round_trip`
* `test_env_ctx_set_overwrite_keeps_latest`
* `test_env_ctx_set_isolation_two_slots`
* `test_env_active_slot_count_after_set_at_least_one`

Tests for the out-of-range guards (`negative_slot_returns_zero` etc.)
PASS — those exercise the source-level early-returns BEFORE the
broken `ctx_set` call would land.  That's defence-in-depth in the
source paying off: the guards are sound regardless of the binding
state.

### §B — overflow guard at `ctx_bridge.vr:106` depends on `Int.MAX` constant

The guard `count > (Int.MAX - 8) / 16` prevents the offset computation
`(count - 1) * 16 + 8` from overflowing.  This is sound but depends on
`Int.MAX` being the platform Int max (typically i64::MAX).  If Verum
ever changes Int to a different width (e.g., wraps at 32-bit on a
target), the guard becomes incorrect.  Recommend: replace with
`saturating_mul` arithmetic + check, OR pin via a refinement type on
the public surface.

### §C — `env_active_slot_count` is O(CONTEXT_SLOT_COUNT) per call

`ctx_bridge.vr:80-90` iterates 0..CONTEXT_SLOT_COUNT and calls
`ctx_get(i).is_some()` once per slot.  Spawn calls this once per
thread creation to size the slots buffer — for CONTEXT_SLOT_COUNT=64
that's 64 TLS reads per spawn.  Recommend: maintain a per-state
`active_count` cell incremented on `ctx_set` to a previously-empty
slot, decremented on `ctx_end` of a present slot.  Brings spawn
overhead down by ~64x on the slot-count path.

### §D — `env_install_parent_contexts` slot-id payload validation

Source validates `slot_id >= 0 && slot_id < CONTEXT_SLOT_COUNT` per
entry (defence in depth against corrupted slot buffer).  Pinned.
Test surface: parent-context-install with mixed valid + invalid
slot entries — verify the valid ones land and invalid ones are
silently dropped.  Gated on `load_i64` intrinsic exercise under
--interp (need a real slots buffer).

## Action items landed in this branch

* `core-tests/runtime/ctx_bridge/unit_test.vr` — 19 unit tests
  covering out-of-range guards + round-trip + isolation + overflow
  + slot-count consistency.
* `core-tests/runtime/ctx_bridge/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| §A unify `env_ctx_set/get` with the CtxProvide/CtxGet opcode path | `crates/verum_vbc` + `core/runtime/ctx_bridge.vr` | 1 day |
| §B refinement-typed `count` parameter | `core/runtime/ctx_bridge.vr` | 30 min |
| §C per-state `active_count` cache | `crates/verum_vbc` + `core/runtime/ctx_bridge.vr` | 1 day |
| §D parent-context partial-validity round-trip test | this folder | gated on safe-buffer harness |
| AOT CtxGet/CtxProvide/CtxEnd cross-tier validation | `crates/verum_codegen/tests/` | 2 h |

## 2026-07-14 session findings

### §E — overflow-guard test SIGSEGV'd the WHOLE in-process runner (test bug + harness finding)

The historical `test_env_install_parent_contexts_overflow_guard` passed
`count = 1_000_000_000_000` believing it exceeded the source guard
`count > (Int.MAX - 8) / 16`.  The threshold is ≈ 5.76e17 — the guard
correctly did NOT fire, the copy loop ran, and the first
`load_i64(0xDEADBEEF)` wild-load SIGSEGV'd the interpreter *process*:
every suite scheduled after ctx_bridge was silently never run.  Two
resolutions landed:

1. Test fix: the count is now computed from the guard's own expression
   (`(Int.MAX - 8) / 16 + 1`) — the guard fires, the call is a no-op.
2. Harness finding escalated as **TEST-RUNNER-ISOLATION-1**: a wild
   raw load in ANY test aborts the whole `--interp` run (single
   process, `RawLoadI64` checks only `addr > 0`).  Options under
   design: per-test subprocess quarantine, sigaction recovery
   trampoline around the dispatch loop, or both.

### §A refinement — root cause narrowed to CTX-STORE-AUTHORITY-1

`env_ctx_set/get` dead round-trip is NOT an intrinsic-binding gap: the
chain reaches `sys.darwin.tls.ctx_get/ctx_set`, which operate on the
TCB-held `ContextSlots` — and `get_current_tcb()` returns `Maybe.None`
under `--interp` (no TCB bootstrap), so sets error out silently and
gets read `Maybe.None`.  Worse, in `env_active_slot_count` the
`ctx_get(i)` result arrives as raw `<nil>` (not a `Maybe` variant) and
`.is_some()` PANICS on the nil receiver — those two tests are pinned
`@ignore` on this.  The fundamental fix is ONE store authority for the
context system and the user-callable surface (task filed:
CTX-STORE-AUTHORITY-1).
