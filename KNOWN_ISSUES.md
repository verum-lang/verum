# Known Issues

## AOT Compilation Non-Deterministic Crash (macOS aarch64)

**Status:** Open (pre-existing, not caused by recent integration work)

**Symptoms:**
- `verum build FILE.vr` or `verum run --aot FILE.vr` exits with SIGSEGV (exit 139) or SIGBUS (exit 138) approximately 40-60% of the time.
- The compiled binary itself works correctly when run directly.
- The crash occurs inside the `verum` driver process during or after LLVM codegen, NOT in user code.

**Root Cause:**
Z3 global-context teardown racing with Rayon ThreadPool shutdown and CVC5 TermManager cleanup. Z3 0.19 uses a process-global context; the Rayon thread pool in `ParallelSolver` spawns workers holding Z3 solver instances. Their destructors race with the main thread's LLVM module cleanup.

**Workaround:**
- Retry the build: the crash is non-deterministic, so a second attempt usually succeeds.
- Use `verum run --interp FILE.vr` for the VBC interpreter path, which is 100% stable.
- For CI: wrap `verum build` in a retry loop.

**Affected Platforms:** macOS aarch64 (Apple Silicon). Occurrence rate on Linux is unknown.

## Feature Flags — All 51 Wired

**All 51** `verum.toml` feature flags are now wired from config through
the compilation pipeline to the subsystem that consumes them. Setting
any key in `verum.toml` or via `-Z` on the CLI has observable effect.

## GPU in Interpreter Mode

When running code with `@device(gpu)` via the VBC interpreter (`verum run --interp`), GPU operations return CPU fallback stubs (e.g., `EnumerateCuda` returns an empty list). This is by design — the interpreter has no GPU hardware access. AOT compilation via `verum run --aot` or `verum build` produces real GPU binaries when MLIR/CUDA/Metal toolchains are available.
