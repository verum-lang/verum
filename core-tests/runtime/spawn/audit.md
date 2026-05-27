# `runtime/spawn` audit

Module: `core/runtime/spawn.vr` (1086 LOC) — SupervisorSpawnConfig +
SpawnConfigBuilder + PriorityLevel + InlineContextStorage + 7
`public fn spawn_*` helpers (spawn_with / spawn_with_retry /
spawn_high_priority / spawn_background / spawn_with_timeout /
spawn_named / spawn_isolated).

Tests: 28 unit tests over PriorityLevel 5-variant + .to_u8 / .from_u8
round-trip + InlineContextStorage + SupervisorSpawnConfig default
values.  Live spawn_* helpers require working async executor +
ExecutionEnv intrinsics — deferred to `vcs/specs/L2-standard/runtime/
spawn/`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.async.spawn` (top-level user-facing) | dispatches to `spawn_with(config, future)` for fine-grained config. |
| `core.runtime.supervisor.Supervisor.start_child` | takes a `SupervisorSpawnConfig` per child. |
| `core.runtime.mod.init_with` | passes through to the runtime-config spawn path. |
| `core.async.runtime` (FullRuntime + SingleThreadRuntime) | uses `PriorityLevel.to_u8` for scheduling-queue insertion. |

## 2. Crate-side hardcodes

| Site | What it pins | Risk |
|---|---|---|
| `@repr(UInt8)` on PriorityLevel | wire-format ordinal (NOT the to_u8 value) | Two distinct numeric surfaces: the @repr ordinal (0..4) and the to_u8 scheduling-weight (32/64/128/192/255). Drift between them surfaces as priority inversion in the scheduler. |
| `@repr(C)` on InlineContextEntry | C-compatible 24-byte layout for FFI to native code | Field order MUST match the codegen-emitted ContextSlot layout. |
| `@size(24)` on InlineContextEntry | exact 24-byte size guarantee | Failure to maintain this is a compile-time error per the @size attribute. |
| 4-entry inline-context-storage limit | hardcoded slot count | A workload that consistently overflows 4 contexts pays the heap-list allocation cost on every spawn; not exposed as a tuning knob. |
| `to_u8` mapping {32, 64, 128, 192, 255} | scheduling-weight per priority | Drift would shift the scheduler's idle/active threshold. |
| `from_u8` range bounds {0..48, 48..96, 96..160, 160..224, 224..256} | decoding-range boundaries | Drift breaks the wire-format round-trip. |

## 3. Language-implementation gaps

### §A — Two numeric surfaces on PriorityLevel (@repr ordinal vs scheduling weight)

`@repr(UInt8)` typically pins the ordinal `{Background=0, Low=1, Normal=2,
High=3, Critical=4}`.  `to_u8()` maps to scheduling weights
`{32, 64, 128, 192, 255}` — DIFFERENT numbers.  This is intentional
(the wire-format ordinal is for serialisation, the scheduling weight
is for the scheduler queue) but the dual mapping is a UX hazard:
"what's the UInt8 representation of Critical?" has two correct
answers.  Recommend: rename `to_u8` → `scheduling_weight` to
disambiguate.

### §B — `from_u8` range-based decoder loses information

`from_u8(150)` returns `Normal` (because 150 ∈ 96..160).  Then
`to_u8(Normal)` returns `128`.  Round-trip is LOSSY for non-canonical
input values.  This is acceptable for the scheduling-weight surface
but problematic if someone uses `to_u8/from_u8` for wire-format
purposes — the implicit lossy round-trip silently corrupts the
wire data.  Recommend: introduce a dedicated `wire_encode` /
`wire_decode` pair that round-trips losslessly (using the @repr
ordinal), and reserve `to_u8` / `from_u8` for the scheduling
weight surface.

### §C — `contexts_overflow: Maybe<List<...>>` — pre-fix dangling reference

Source comment (spawn.vr:286-292) documents a pre-fix defect:
previously `Maybe<&List<...>>` borrowed a temporary that dropped
at end of expression.  Now `Maybe<List<...>>` — owned list.  Pin
in regression_test.vr that the field type is owned, not borrowed
(a future refactor that reverts would re-open the dangling-ref bug).

### §D — `SpawnConfigBuilder.validate_and_build` error accumulation

The builder accumulates errors in `errors: List<Text>` and surfaces
them at build time.  Coarse-grained: caller can't distinguish
"unrecoverable" from "warnable" errors.  Recommend: split into
`errors` + `warnings` lists.

### §E — `InlineContextStorage.push` returns `Bool`, not `Result`

Push silently fails (returns `false`) on overflow.  Caller-side
overflow tracking is brittle.  Recommend: change to
`Result<(), ContextOverflow>` and let the call sites decide
whether to spill to the heap-list or fail.

### §F — `InlineContextStorage.new()` / `SupervisorSpawnConfig.new()` field-write OOB (codegen defect class)

Surfaced 2026-05-27.  Same defect class as
[[btree_pattern_match_ref_generic_class]] /
[[enactment_field_access_oob_2026-05-24]] /
[[use_after_free_error_field_shift_2026-05-27]].

* `InlineContextStorage.new()` panics with
  `field write out of bounds: field index 9 (offset 72+8 = 80) exceeds
  object data size 24`.
* `SupervisorSpawnConfig.new()` reaches the same panic via its
  embedded `InlineContextStorage.new()` call.

Root cause: cross-module ctor returns lose record layout in the
codegen.  The `InlineContextStorage` record contains an array field
`entries: [InlineContextEntry; 4]` that expands to 4 × 24 = 96 bytes
inline, plus `count: UInt8` + `_padding: [Byte; 7]` = total 104 bytes.
The codegen mis-sizes the returned record at 24 bytes (one entry),
and the ctor's `entries[idx] = InlineContextEntry { ... }` write at
the 9th-position field index (after 4 entries + count + 7-byte
padding) overflows the truncated allocation.

Pinned `@ignore` on:
* `test_inline_context_storage_new_is_empty`
* `test_inline_context_storage_new_count_is_zero`
* 7 `test_spawn_config_new_default_*` tests
* `test_spawn_config_alias_resolves_to_supervisor_spawn_config`

Multi-day VBC codegen work — root cause sits at the same
`compile_field_access` / `populate_types_from_archive` chain
[[self_substitution_read_site_2026-05-27]] and
[[task47_stage3_stub_cascade_fix_2026-05-24]] partially close.

## Action items landed in this branch

* `core-tests/runtime/spawn/unit_test.vr` — 28 unit tests covering
  PriorityLevel 5-variant + to_u8/from_u8 boundary table + Default +
  InlineContextStorage + SupervisorSpawnConfig defaults +
  SpawnConfig alias.
* `core-tests/runtime/spawn/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| §A Rename `to_u8` → `scheduling_weight` | `core/runtime/spawn.vr` + callers | 1 h |
| §B Add `wire_encode` / `wire_decode` lossless pair | `core/runtime/spawn.vr` | 1 h |
| §C Pin owned-list invariant in regression_test | this folder | 30 min |
| §D Builder warnings vs errors split | `core/runtime/spawn.vr` | 1 h |
| §E `push` returns Result | `core/runtime/spawn.vr` + InlineContextStorage callers | 2 h |
| Live spawn_with(config, future) round-trip | `vcs/specs/L2-standard/runtime/spawn/` | gated on async executor |
| Builder method chain test (.permanent / .transient / .temporary / .background / .critical) | this folder | 1 h |
