# Known Issues

This file tracks remaining runtime limitations. Items are removed as they
resolve. When the file is empty, it can be deleted.

## Shared<T> Runtime Construction — type-size lowering

**Status:** Partially resolved.

The historical "explicit panic on first `Shared<T>::new`" was a stack of
four bugs, three of which are now fixed:

1. `@thread_local` init ctors not run before main → commit `7e12603`.
2. Arithmetic handlers panicking on non-Int tags → commit `e751045`.
3. **VBC ABI collision**: `FfiSubOpcode::CbgrAlloc` codegened as 0x60 but
   0x60 decodes to `DerefRaw` → commit `50249c3`. New sub-opcodes
   `CbgrAlloc = 0xA0`, `CbgrAllocZeroed = 0xA1`, `CbgrDealloc = 0xA2`
   with interpreter + AOT handlers.
4. `cbgr_alloc`/`cbgr_alloc_zeroed` not annotated as intrinsics, so the
   stdlib bytecode body ran (hitting an FFI-less `mmap` path) → commit
   `2ca249f`. Functions now carry `@intrinsic("cbgr_alloc")` etc. and
   the interpreter handler returns a properly-wrapped `Ok(tuple)` /
   `Err(…)` Result variant.

End-to-end verification: direct `cbgr_alloc(32, 8)` in interpreter mode
now returns `Ok((ptr, gen, epoch))` and user code's `match` destructures
it correctly. `Heap<T>::new`, `Map`, `List` backing allocations travel
the same path and also work.

**Remaining**: `Shared<Int>::new(42)` still trips at the very top of the
call chain — the `SharedInner<T>.size` / `.alignment` access on the
generic type delivers `i64::MAX` into the register instead of the real
size (16/8). Every `.size` / `.alignment` lookup on a concrete
instantiation of a generic struct type mis-routes. The fix needs to
either thread instantiated type-ids through the call-site's register
preparation, or lower `.size` on a generic-type expression to
`SizeOfG(resolved_type_id)` rather than a plain field load.

Single-observer patterns (no `Shared<T>` at all), `Heap<T>::new`, and
direct `cbgr_alloc`/`cbgr_alloc_zeroed` calls work today.

## REPL — Parse-Only

`verum repl` parses and type-checks input but does not evaluate. Full
VBC-backed evaluation (persistent `InterpreterState`, bindings across
prompts) is a tracked follow-up. Use `verum run <file.vr>` for
execution.

## AOT Async — No Polling Executor

`verum build` emits correct async state-machine code but does not link
a future-polling runtime. A binary that reaches an `.await` has no
executor. Use `verum run --interp` (Tier 0) for async code today; a
minimal single-threaded polling executor for AOT is the follow-up.

## GPU / FFI / ML vmap-pmap in Tier 0 Interpreter

By design: the interpreter returns `NotImplemented` for real GPU
dispatch, dynamic FFI resolution, and ML vmap/pmap. Use `verum run
--aot` / `verum build` with the appropriate toolchain for these.
