# Known Issues

This file tracks remaining runtime limitations. Items are removed as they
resolve. When the file is empty, it can be deleted.

## GPU / FFI / ML vmap-pmap in Tier 0 Interpreter

By design: the interpreter returns `NotImplemented` for real GPU
dispatch, dynamic FFI resolution, and ML vmap/pmap. Use `verum run
--aot` / `verum build` with the appropriate toolchain for these.
