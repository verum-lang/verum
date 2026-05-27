# `runtime/stack_alloc` audit

Module: `core/runtime/stack_alloc.vr` (822 LOC) — stack-based + arena +
pool allocators for no-heap / embedded targets.

**`@cfg(any(runtime = "no_heap", runtime = "embedded"))`** — the module
is mounted only under these two profiles.  On the default `runtime =
"full"` profile (which the conformance suite runs against),
`core.runtime.stack_alloc` is **not in scope**.

For this reason, the unit_test surface is intentionally minimal and
defers most coverage to a future no-heap conformance suite at
`core-tests/runtime-noheap/stack_alloc/`.

Tests landed: 0 (data-only types are cfg-gated; mount fails on
default profile).  Audit-only deliverable for this branch.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.mem.alloc` (when `runtime = "no_heap"`) | `global_allocator()` returns a `StackAllocator` / `ArenaAllocator` / `PoolAllocator` per build profile. |
| `core.runtime.config.EmbeddedRuntime` | uses `StackAllocator` as its `RuntimeConfig.Allocator`. |
| Real-time / safety-critical applications | use the type aliases `TinyStackAllocator` (1KB) / `SmallStackAllocator` (4KB) / `MediumStackAllocator` (16KB) / `LargeStackAllocator` (64KB) for known-bound stack scopes. |
| Connection-pool patterns | use `ConnectionPoolArena` (1024-byte × 128 blocks) for pooled connection objects. |

## 2. Surface (gated)

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

### §A — Module-level `@cfg(any(runtime = ...))` gates the whole surface

A test file that tries to `mount core.runtime.stack_alloc.StackSavepoint`
under the default `runtime = "full"` profile fails with a
ModuleNotFound at compile time.  The cfg gate is correct (these
allocators are meaningless under a full runtime with heap) but it
means the conformance suite can't exercise the data-only ADT surface
under the same conditions as the rest of `core-tests/runtime/`.

Recommendation: split off the data-only types (StackSavepoint,
ArenaSavepoint) into a non-cfg-gated `core/runtime/stack_alloc/types.vr`
submodule that's always in scope.  The allocator implementations stay
gated.

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
| §A Split data-only types into non-cfg-gated submodule | `core/runtime/stack_alloc.vr` | 1 day |
| §B Instrumentation cfg-gate | `core/runtime/stack_alloc.vr` | 30 min |
| §C Refinement-typed const-generic parameters | `core/runtime/stack_alloc.vr` | gated on language feature |
| §D Use UInt64.MAX as freelist end-marker | `core/runtime/stack_alloc.vr` | 15 min |
| Full no-heap conformance suite | `core-tests/runtime-noheap/stack_alloc/` | 1 week (gated on §A) |
| Live ScopedStack RAII drop test | future no-heap suite | gated |
| Pool freelist round-trip test | future no-heap suite | gated |
