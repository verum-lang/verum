# Known Issues

This document tracks outstanding runtime limitations and known-incomplete
features. Resolved entries are moved to `vcs/baselines/` and the git log.

## AOT Compilation Stability (macOS aarch64)

**Status:** Substantially improved (`make test-l0` previously SIGABRT'ed at
spec ~400; now proceeds through the full suite). Two fundamental AOT
codegen fixes landed in this cycle:

1. **Slice-op fat-ref loads** (commit 04de418) — `SliceGet`,
   `SliceGetUnchecked`, `SliceSubslice`, `SliceSplitAt` unconditionally
   called `.into_pointer_value()` on the register value; a register
   holding a NaN-boxed i64 encoding of the pointer panicked. All four
   sites now route through `as_ptr(ctx, val, name)` which handles
   PointerValue / IntValue / StructValue.
2. **Arity-collision fixups** (commit 238624e, prior cycle) — stdlib
   functions with arity collisions got null Type* references that
   crashed LLVM passes. Skip-body stubs + `optnone`/`noinline` contain
   the damage; the `~4%` residual under heavy concurrent I/O is
   addressed by running under `codegen-units=1` (default release).

**Remaining surface**: 57 other `.into_pointer_value()` call sites in
`verum_codegen/src/llvm/instruction.rs`. Any of them taking register
values from `ctx.get_register(...)` are latent candidates for the same
bug. VCS differential specs surface them when they trigger.

## Feature Flags — All 51 Wired

All 51 `verum.toml` feature flags are wired from config through the
compilation pipeline to the consumer subsystem. Setting any key or
passing `-Z key=value` on the CLI has an observable effect.

## GPU / FFI / ML vmap-pmap in Tier 0 Interpreter

By design: the Tier 0 interpreter returns `NotImplemented` for GPU
kernel dispatch, FFI symbol resolution, and ML vmap/pmap transforms.
These features require AOT (`verum run --aot` / `verum build`) with
MLIR/CUDA/Metal toolchains for GPU, `dlopen`/`dlsym` for FFI, and a
distributed runtime for pmap. The interpreter errors loudly rather
than silently producing fake handles, so callers see the boundary.

`IsSymbolResolved` correctly returns `false` in the interpreter so
conditional branches on FFI availability work.

## Shared<T> Runtime Construction — Partially Resolved

**Status:** The `@thread_local static` initializer pathway
(`__tls_init_*` functions) was not being invoked by the interpreter —
the pipeline wholesale-skipped `module.global_ctors` as a defense
against FFI library initializers that crash on macOS. That caused
`static mut LOCAL_HEAP: Maybe<LocalHeap> = None` to read back as a
raw zero `Value` instead of the `None` variant, and the CBGR
allocator bootstrap crashed.

Fixed in commit 7e12603: the interpreter now executes the TLS subset
of ctors (function name starts with `__tls_init_`) before main, so
`@thread_local static mut COUNTER: Int = 42` reads back as `42`. FFI
initializers keep their existing skip.

**Remaining:** `Shared::new(42)` still panics further down the
allocation path with "Expected int, got None" at `value.rs:892`
inside `integer_arith::handle_subi`. The crash is no longer in the
TLS bootstrap but in a subsequent `cbgr_alloc` / `LocalHeap` step.
Single-observer `Cancellation` patterns that avoid `Shared<T>` work;
multi-clone tokens still require this fix.

## Variant Method Dispatch

Fixed (commit 94b16bf). Direct method calls on Maybe / Result
(`.unwrap`, `.unwrap_or`, `.is_some`, `.is_none`, `.is_ok`, `.is_err`,
`.take`, `.as_ref`, `.as_mut`, `.ok`, `.expect`, `.unwrap_err`)
resolve correctly in both interpreter and AOT.

## Text Builder + Byte-Level Writes

Fixed (commit 945753f, b26abf8, 270a8e6, this cycle). The stdlib
Text builder uses `memset(ptr, byte, 1)` instead of the generic
`ptr_write<T>` intrinsic. `memset` lowers to
`FfiSubOpcode::CMemset` → `std::ptr::write_bytes(ptr, byte, 1)` on
the interpreter and to `llvm.intr.memset` on AOT — both byte-sized.
`Text == Text` equality and hashing honor the builder layout, so
`Map<Text, V>` works on keys built incrementally.

## Cache Invalidation

The stdlib disk cache at `target/.verum-cache/stdlib/` is keyed by
compiler version, LLVM version, and a blake3 hash of every
`core/*.vr` file. Upgrading the compiler or LLVM invalidates the
cache automatically.

## REPL — Parse-Only

`verum repl` parses and type-checks input but does not evaluate:
`process_input` at `crates/verum_cli/src/commands/repl.rs:231` only
reports the parse result. Full VBC evaluation in the REPL is a
tracked follow-up (wiring a persistent `InterpreterState` that
accumulates bindings across prompts).

For execution, use `verum run <file.vr>`.

## AOT Async — No Polling Executor

`verum build` on a program that uses `async`/`await`/`spawn` emits
correct state-machine code but produces a standalone C-ABI binary
without a future-polling runtime linked. Running such a binary
reaches an `await` and has no executor to drive the future forward.

For async code use `verum run --interp` (Tier 0 interpreter has an
internal driver). Adding a minimal single-threaded polling executor
to AOT binaries is tracked as a separate task.
