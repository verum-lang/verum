# Known Issues

## AOT Compilation Stability (macOS aarch64)

**Status:** Fixed (commit 238624e). Previously crashed ~40-60% of runs;
now **96-100% stable** (50/50 foreground, 96/100 under heavy load).

**Root cause:** VBC→LLVM lowering emitted stdlib functions with null
Type* references from arity-collision fixups. LLVM passes (Verifier,
TypeFinder, SelectionDAG, InterleavedAccess) crashed on these.

**Fix:** Skip-body functions now get trivial `ret zeroinitializer` stubs,
stdlib functions get `optnone` + `noinline` attributes, LLVM verification
and IR dump are gated, and the pass pipeline is restricted when the
module has known issues.

**Residual:** Under extreme resource pressure (~4% of runs with
concurrent heavy I/O), the LLVM compilation may still fail. Use the
VBC interpreter path (`verum run --interp`) as fallback.

## Feature Flags — All 51 Wired

**All 51** `verum.toml` feature flags are now wired from config through
the compilation pipeline to the subsystem that consumes them. Setting
any key in `verum.toml` or via `-Z` on the CLI has observable effect.

## GPU in Interpreter Mode

When running code with `@device(gpu)` via the VBC interpreter
(`verum run --interp`), GPU operations return CPU fallback stubs
(e.g., `EnumerateCuda` returns an empty list). This is by design —
the interpreter has no GPU hardware access. AOT compilation via
`verum run --aot` or `verum build` produces real GPU binaries when
MLIR/CUDA/Metal toolchains are available.

## FFI in Interpreter Mode

The Tier 0 interpreter does not perform dynamic symbol resolution
(`dlopen`/`dlsym`) or ABI-aware marshalling. The corresponding
opcodes (`LoadSymbol`, `GetLibrary`, `ArrayFromC`, `StructToC`,
`StructFromC`) return `NotImplemented` errors rather than silently
producing fake handles or mismarshalled data. Use `--aot` for FFI.

`IsSymbolResolved` correctly returns `false` in the interpreter,
which is the correct semantic — callers that branch on this value
skip the FFI path as intended.

## ML Vectorization / Distributed Compute

`VmapTransform` and `PmapTransform` require JIT tracing and a
distributed runtime respectively, neither of which the interpreter
provides. These opcodes return `NotImplemented` instead of the
previous behavior of silently returning nil. Use AOT mode with
an appropriate runtime for these.

## Shared&lt;T&gt; / CBGR-allocator Bootstrap (architectural)

`Shared.new(...)` and other CBGR-tracked reference primitives fail
at runtime in both interpreter and AOT because the stdlib memory
allocator functions (`get_heap`, `cbgr_alloc`, `LocalHeap.new`) are
not linked into user modules via the current mount/link system.

**Symptom:** `Shared.new(struct)` → "field write out of bounds:
field index 257" in the interpreter; `undefined function: get_heap`
in AOT. The "field out of bounds" is cascading from an uninitialized
allocator state rather than a real bytecode bug.

**Root cause:** The stdlib bootstrap loads `core/mem/*.vr` into the
module registry for type checking but does not emit their function
bodies into the user's VBC module. When user code transitively
depends on `cbgr_alloc` via `Shared.new`, the call site is present
but the callee has no body.

**Impact:** Stdlib types that wrap `Shared<Inner>` (Channel,
Broadcast, Oneshot, CancellationToken) typecheck and compile but
crash at construction. Their public APIs are complete and ready;
runtime semantics unblock when the bootstrap is fixed.

**Workaround:** Use direct `alloc()` intrinsic in user code rather
than CBGR-tracked allocation. This bypasses the allocator bootstrap
at the cost of losing per-object generation/epoch tracking.

**Fix path:** Extend `clear_non_compilable_stdlib_modules` in
`pipeline.rs` to retain `core.mem.*` modules for codegen, or
convert CBGR allocator functions to compiler intrinsics with direct
VBC opcode lowering.

## Cancellation Tokens

`core.async.cancellation` is API-complete and typechecks. Runtime
construction is blocked by the allocator bootstrap above.

## Cache Invalidation

The stdlib disk cache at `target/.verum-cache/stdlib/` is keyed by
compiler version, LLVM version, and a blake3 hash of every
`core/*.vr` file. Upgrading the compiler or LLVM invalidates the
cache automatically — no manual `cargo clean` needed after version
bumps.
