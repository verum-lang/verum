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

## Feature Flags Not Yet Consumed

The following `verum.toml` configuration keys are accepted and validated but do **not** yet change compiler behavior. Setting them has no effect:

- `[types]`: `dependent`, `higher_kinded`, `universe_polymorphism`, `coinductive`, `quotient`, `instance_search`, `coherence_check_depth`
- `[runtime]`: All 8 fields (`cbgr_mode`, `async_scheduler`, `async_worker_threads`, `futures`, `nurseries`, `task_stack_size`, `heap_policy`, `panic`)
- `[codegen]`: `gpu_backend`, `monomorphization_cache`, `debug_info`, `tail_call_optimization`, `vectorize`, `inline_depth`
- `[meta]`: `compile_time_functions`, `quote_syntax`, `macro_recursion_limit`, `reflection`, `max_stage_level`
- `[protocols]`: All 6 fields
- `[context]`: `unresolved_policy`, `negative_constraints`, `propagation_depth`
- `[safety]`: `ffi_boundary`, `capability_required`, `mls_level`, `forbid_stdlib_extern`
- `[test]`: `differential`, `property_testing`, `proptest_cases`, `fuzzing`, `parallel`
- `[debug]`: `step_granularity`, `inspect_depth`, `show_erased_proofs`

**Active flags (change behavior):** `types.refinement`, `types.cubical`, `codegen.tier`, `codegen.proof_erasure`, `meta.derive`, `safety.unsafe_allowed`, `safety.ffi`, `context.enabled`, `debug.dap_enabled`, `debug.port`, `test.timeout_secs`, `test.deny_warnings`, `test.coverage`.

## GPU in Interpreter Mode

When running code with `@device(gpu)` via the VBC interpreter (`verum run --interp`), GPU operations return CPU fallback stubs (e.g., `EnumerateCuda` returns an empty list). This is by design — the interpreter has no GPU hardware access. AOT compilation via `verum run --aot` or `verum build` produces real GPU binaries when MLIR/CUDA/Metal toolchains are available.
