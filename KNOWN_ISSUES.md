# Known Issues

This file tracks remaining runtime limitations. Items are removed as they
resolve. When the file is empty, it can be deleted.

## AOT Async — No Polling Executor

`verum build` emits correct async state-machine code but does not link
a future-polling runtime. A binary that reaches an `.await` has no
executor. Use `verum run --interp` (Tier 0) for async code today; a
minimal single-threaded polling executor for AOT is the follow-up.

## GPU / FFI / ML vmap-pmap in Tier 0 Interpreter

By design: the interpreter returns `NotImplemented` for real GPU
dispatch, dynamic FFI resolution, and ML vmap/pmap. Use `verum run
--aot` / `verum build` with the appropriate toolchain for these.
