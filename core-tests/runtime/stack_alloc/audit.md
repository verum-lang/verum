# `runtime/stack_alloc` audit

Module: `core/runtime/stack_alloc.vr` (822 LOC) — stack-based + arena +
pool allocators for no-heap / embedded targets.

**2026-07-14 — §A RESOLVED AT THE ROOT**: the module-level
`@cfg(any(runtime = "no_heap", runtime = "embedded"))` gate in
`core/runtime/mod.vr` was REMOVED (not split, as §A originally
proposed).  Rationale: the allocators are self-contained (fixed
`[Byte; SIZE]` buffers, `mem.{Alloc, Layout, AllocError}` +
`sync.AtomicInt` only — no OS dependency), so gating them out of the
full runtime was pure surface loss: it made the module untestable
under the default conformance profile AND withheld
deterministic-latency allocators (request arenas, connection pools)
from hosted targets.  PRIMARY use remains no-heap/embedded
(EmbeddedRuntime's allocator), now as the default-profile-visible
general-purpose tool it always was.

Tests landed 2026-07-14: `unit_test.vr` — accounting contract
(capacity/used/remaining/watermark/alloc_count), savepoint rewind,
OOM error surface (no-panic + no-state-mutation), alignment law,
pool block recycling.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.mem.alloc` (when `runtime = "no_heap"`) | `global_allocator()` returns a `StackAllocator` / `ArenaAllocator` / `PoolAllocator` per build profile. |
| `core.runtime.config.EmbeddedRuntime` | uses `StackAllocator` as its `RuntimeConfig.Allocator`. |
| Real-time / safety-critical applications | use the type aliases `TinyStackAllocator` (1KB) / `SmallStackAllocator` (4KB) / `MediumStackAllocator` (16KB) / `LargeStackAllocator` (64KB) for known-bound stack scopes. |
| Connection-pool patterns | use `ConnectionPoolArena` (1024-byte × 128 blocks) for pooled connection objects. |

## 2. Surface

| Type | Const params | Purpose |
|---|---|---|
| `StackAllocator<const SIZE: Int>` | total buffer size | bump allocator with watermark + alloc count |
| `StackSavepoint` | — | (top, alloc_count) snapshot for rewind |
| `ArenaAllocator<const CHUNK_SIZE: Int, const MAX_CHUNKS: Int>` | per-chunk size + max chunks | bulk-deallocation by-region |
| `ArenaSavepoint` | — | (chunk, pos, total) snapshot |
| `PoolAllocator<const BLOCK_SIZE: Int, const BLOCK_COUNT: Int>` | per-block size + count | fixed-size O(1) alloc/free |
| `ScopedStack<const SIZE>` / `ScopedArena<...>` | — | RAII wrappers |
| Type aliases: `TinyStackAllocator`=`StackAllocator<1024>` | — | preset sizes |

## 3. Crate-side hardcodes

| Site | What it pins | Risk |
|---|---|---|
| Bump pointer arithmetic on `buffer[top..top+size]` | byte-level slice indexing within the fixed buffer | LLVM optimisation here is critical for the "~0ns" claim — drift surfaces as 100ns+ alloc overhead. |
| Freelist encoding in `PoolAllocator.blocks[i][0..8]` | little-endian UInt64 next-index | Endianness drift between codegen and source surface silently corrupts the freelist. |
| Watermark + alloc_count fields | runtime instrumentation | Disabled-via-`@cfg(no_instrumentation)` is the production path. |

## 4. Language-implementation gaps

### §A — CLOSED 2026-07-14 (cfg gate removed at `core/runtime/mod.vr`)

Original finding: the module-level cfg gate made the whole surface
untestable under the default profile.  Resolution went further than
the proposed data-only split: the gate itself was removed, because the
implementation has no platform dependency and the "meaningless under a
full runtime" premise was wrong — deterministic-latency allocation is
a hosted-target need too.  The no-heap/embedded profiles see exactly
the same module.

### §B — `@cfg(no_instrumentation)` watermark/alloc_count gate

Fields are unconditional in source; performance-critical builds
should gate them behind `@cfg(stack_alloc_instrumentation)` to drop
them from the production ABI.

### §C — Const-generic surface lacks compile-time alignment check

`StackAllocator<7>.new()` compiles even though a 7-byte buffer is
never useful (sub-word alignment hazard).  Refinement-typed `SIZE`
parameter (`SIZE: Int { SIZE >= 64 && SIZE % 8 == 0 }`) would
catch this at compile.

### §D — `PoolAllocator.blocks[BLOCK_COUNT - 1]` end-marker freelist hack

Source `stack_alloc.vr:613` uses `BLOCK_COUNT` as the
"end of freelist" sentinel.  Defence in depth would use
`UInt64.MAX` (the canonical sentinel for index-based freelists)
to avoid collision with a future `BLOCK_COUNT >= 2^64` configuration.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| §B Instrumentation cfg-gate | `core/runtime/stack_alloc.vr` | 30 min |
| §C Refinement-typed const-generic parameters | `core/runtime/stack_alloc.vr` | gated on language feature |
| §D Use UInt64.MAX as freelist end-marker | `core/runtime/stack_alloc.vr` | 15 min |
| Live ScopedStack RAII drop test | needs `&mut` field-holding record support validation | small |
| Pool freelist round-trip byte-level test | `core-tests/runtime/stack_alloc/` | small (now ungated) |
