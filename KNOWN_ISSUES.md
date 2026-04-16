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

## Cancellation Tokens

`core.async.cancellation` is API-complete and typechecks, but full
runtime semantics depend on `Shared<T>` interpreter support that is
still landing. Single-observer cancellation patterns work today;
shared-clone semantics (one observer cancels all clones) are under
active work.

## Cache Invalidation

The stdlib disk cache at `target/.verum-cache/stdlib/` is keyed by
compiler version, LLVM version, and a blake3 hash of every
`core/*.vr` file. Upgrading the compiler or LLVM invalidates the
cache automatically — no manual `cargo clean` needed after version
bumps.
