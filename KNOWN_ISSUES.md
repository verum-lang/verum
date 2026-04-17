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

## Shared&lt;T&gt; / CBGR-allocator Bootstrap (partially resolved)

**Status:** Partially fixed (commit 871cf9d). Added `core.mem.*`
modules to the AOT retention list so the CBGR allocator bodies are
now linked into user modules. The "field write out of bounds:
field index 257/271" cascade failure — which was the surface
symptom — is fully resolved. The remaining gap is narrower:

**Remaining:** Runtime construction of `Shared::new(...)` still
crashes with "explicit panic" because the CBGR allocator's
bootstrapped global heap (`CURRENT_HEAP`) depends on thread-local
init via `@thread_local static`, which the interpreter does not
yet fully support. AOT mode has the same limitation for this path.

**Workaround:** avoid user code that constructs `Shared<T>` in
interpreter mode. Single-observer patterns (direct `&T`,
`CancellationFlag` without Shared wrapping) work today.

## Cancellation Tokens

`core.async.cancellation` is API-complete and typechecks. The
runtime `Shared<T>` construction path (used for clonable tokens)
is blocked on the thread-local static bootstrap above. Single-
observer patterns work via the exposed `CancellationFlag` type.

## Variant Method Dispatch

**Status:** Fixed (commit 94b16bf). Direct method calls on
Maybe/Result (`.unwrap`, `.unwrap_or`, `.is_some`, `.is_none`,
`.is_ok`, `.is_err`, `.take`, `.as_ref`, `.as_mut`, `.ok`,
`.expect`, `.unwrap_err`) now work correctly in the interpreter.

**Background:** `core.base.maybe` is intentionally excluded from
compilation to avoid type collisions with user-defined `Maybe`.
The LLVM AOT path has an inline intercept in `instruction.rs:9785`;
the interpreter now mirrors that in `dispatch_variant_method` using
empirical variant-layout heuristics (Some/Ok = tag 0 with payload;
None = tag 0 no payload; Err = tag 1 with payload).

## Cache Invalidation

The stdlib disk cache at `target/.verum-cache/stdlib/` is keyed by
compiler version, LLVM version, and a blake3 hash of every
`core/*.vr` file. Upgrading the compiler or LLVM invalidates the
cache automatically — no manual `cargo clean` needed after version
bumps.
