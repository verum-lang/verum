# `core.mem.arena` — audit findings

> Module under test: `core/mem/arena.vr` (643 LOC; 5 constants
> (DEFAULT/MAX capacity, growth factor, alignment, GEN_INITIAL),
> 1 record `ArenaConfig`, 1 sum type `ArenaError`, 1 record
> `GenerationalArena`, 1 record `ArenaSnapshot`).
>
> Test surfaces (this branch):
> `unit_test.vr` (~75 LOC), `property_test.vr` (~40 LOC),
> `integration_test.vr` (~60 LOC), `regression_test.vr` (~40 LOC).
>
> Tests pin only the static-shape contract — constants, config record
> construction, arena-vs-header generation lattice consistency. Live
> arena allocate / reset / snapshot tests are deferred to a future
> integration suite (require deliberate state management).

## 1. Cross-stdlib usage

The generational arena is the load-bearing per-request / per-frame
allocator in:

| Consumer | Use |
|---|---|
| Parser (future) | AST nodes allocated in arena; bulk-invalidated on error recovery. |
| Async tasks | Per-task arenas for request-scoped data. |
| Game-engine pattern | Per-frame allocator, reset at frame boundary. |
| GPU command staging | Draw-command arenas, reset per render pass. |

## 2. Crate-side hardcodes

Drift surfaces:

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `DEFAULT_ARENA_CAPACITY = 64 KiB` | Default initial size | Caller-tunable; default must be sensible. |
| `MAX_ARENA_CAPACITY = 256 MiB` | Hard cap on unbounded growth | Drift past 256 MiB would let runaway allocations exhaust process memory. |
| `ARENA_ALIGNMENT = 8` | 64-bit word alignment | u64 / pointer fields rely on this. |
| Arena's `generation` field uses same GEN_* lattice as headers | CBGR consistency | Pinned by integration_test §2. |

## 3. Language-implementation gaps

### 3.1 Live arena allocator integration not exercised

`GenerationalArena.new` / `.alloc` / `.reset` / `.snapshot` require
live OS-memory integration. Tests in this branch pin the constants
and config-record shape; live tests are deferred.

### 3.2 Snapshot-based rollback is an open design

`ArenaSnapshot` captures (used, generation) for partial rollback.
The semantics interact with CBGR's generation-bump invariant —
rolling back the `used` counter without bumping generation lets
old refs still validate. Currently the API only supports full
reset (which bumps generation). Future work: rollback via local
generation increment that observers can detect.

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| 1 | Missing test coverage for `core/mem/arena.vr` | `core-tests/mem/arena/{unit,property,integration,regression}_test.vr` | 4-file suite; ~215 LOC total (static-shape only). |
| 2 | Missing `audit.md` for `core-tests/mem/arena/` | This file. |
| 3 | **DEFECT SURFACED** — `GenerationalArena` constructors fail. Tracked as **task #8**. 9 live-lifecycle tests + 2 minimal repros in `integration_test.vr` / `regression_test.vr` pinned `@ignore` in their final form; they flip green automatically when task #8 closes. |

### §4.3 — Task #8 isolation steps (recorded in `integration_test.vr §0` probes)

| Probe | Result | What it tells us |
|---|---|---|
| `probe_arenaconfig_default_field_access` | ✅ pass | Cross-module 3-field stdlib record constructor + field access works |
| `probe_arenaconfig_fixed_field_access` | ✅ pass | Same for `.fixed(N)` constructor |
| `probe_user_seven_field_record_field_access` | ✅ pass | User-defined 7-Int-field record + inline literal works |
| `probe_user_record_with_nested_constructor_field` | ✅ pass | User record with a slot built via `ArenaConfig.default()` works |
| `probe_generationalarena_direct_field_read` | ❌ fail | `GenerationalArena.new(N)` returns non-pointer; first GetF on result errors NullPointer |
| `probe_arena_with_config_constructor` | ❌ fail | `GenerationalArena.with_config(...)` errors inside its own body at SetF (opcode 0x63) PC 88 |

The failure differs by constructor — `.new(N)` returns a malformed
Value (caller-side GetF errors), `.with_config(cfg)` errors mid-body
at a SetF on a null receiver.  Together they indicate the
**MakeRecord step for GenerationalArena is broken** — most likely
because the precompiled-stdlib archive registers
`GenerationalArena` with the wrong field count (or wrong
`type_field_layouts`).  Repro one-liner: `let a =
GenerationalArena.new(1024); let _ = a.capacity;` — sufficient for
the verum_vbc dev loop.  Same defect class as the closed List /
Map / BinaryHeap layout-drift fixes (search `core-tests/INVENTORY.md`
for "Two fundamental fixes CLOSED in this branch" entries).

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Live `GenerationalArena.new / alloc / reset / clone` round-trip. | Blocked on **task #8** (~2-3 days, fundamental fix in record-receiver method dispatch path). 9 tests written in final form, pinned `@ignore`. |
| §B | Test arena snapshot / restore. | Blocked on §A. |
| §C | Test ArenaError variants (CapacityExceeded, AllocationTooLarge, BufferOverflow). | ~30 min | open |
| §D | Cross-tier divergence sweep on `--aot` + `--interp`. | 1 hour wall-clock | open |
