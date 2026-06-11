# `core.sys.durability` — implementation audit

## Status: **complete** (2026-06-11) — 11/11 `--interp` after Bug A + Bug B closed

> **2026-06-11: 11/11 pass under `--interp`** (commits `cee79ec03` Bug A +
> `08ede3518` Bug B). The error-funnel `@test`s below now correctly
> observe `full_fsync(FileDesc(-1))` → `Err(EBADF)`.
>
> History (now fixed): these tests had pinned a real live defect — at
> stdlib precompile, `sys.common.full_fsync`'s `@cfg`-dispatched
> cross-module call to `sys.darwin.libsystem.safe_full_fsync` (and
> `sync_directory`'s `safe_open`/`safe_fsync`/`safe_close`) compiled to
> `LOAD_NIL; RET` (**Bug A** — rooted cross-module call not resolved to
> the task-#47 unique free-fn stub), and even when un-stubbed the FFI
> symbol failed to resolve in the consuming module (**Bug B** — archive
> FFI symbols not carried over on body-merge). Both closed; see
> `core-tests/sys/SYS_SPECTRUM_AUDIT.md` §A/§B.

* `core.sys.durability` is a thin re-export surface over
  `core.sys.common` — see `core/sys/durability.vr:33-43`. The behaviour
  is owned by `common.vr` (`full_fsync` / `data_only_fsync` /
  `sync_directory` / `pread` / `pwrite`); this module exists to
  give callers an intent-named import surface.
* `unit_test.vr` and `property_test.vr` pin the error-funnel
  semantics on invalid fds across the full fsync family. **These
  currently FAIL** (see baseline note above) — they correctly pin a
  real, live stdlib defect rather than a passing contract.
* The **happy-path fsync round-trip** (open + write + full_fsync +
  close + reopen + read back) is **deferred** — same fixture
  requirement as the `file_ops` happy-path round-trip.
* `pread` / `pwrite` / `sync_directory` happy-paths are deferred
  because their CBGR-byte-slice receiver surfaces need the
  `core.io.fs` integration layer to set up.

## 1. Cross-stdlib usage

`core.sys.durability` is consumed by `core.io.fs`, `core.database.*`,
and any code that needs intent-named crash-safety:

| Consumer | Touches | Notes |
|---|---|---|
| `core/io/fs.vr` | `full_fsync` / `sync_directory` | rename-survival path. |
| `core/database/sqlite/*` | `full_fsync` / `pread` / `pwrite` | SQLite native backend. |
| `core/storage/wal.vr` | `data_only_fsync` | WAL flush. |

## 2. Crate-side hardcodes

Same as `core.sys.common` — the implementations live in
`crates/verum_codegen/src/llvm/platform_ir.rs` and the FFI shims at
the per-platform layer. No additional hardcodes for the
intent-named re-export.

## 3. Language-implementation gaps surfaced by this suite

### 3.1 `public mount X.Y` parent-prefix scan (already closed)

* The intent-named re-export at `core/sys/durability.vr:36-42`
  goes through the same `process_import_tree` parent-prefix scan
  that `cabi.vr` / `common.vr` use. Pinned here against re-regression
  in `regression_test.vr` §A.

## 4. Action items landed in this branch

1. `unit_test.vr` — 3 `@test`s covering the invalid-fd Err funnel
   across `full_fsync` / `data_only_fsync`.
2. `property_test.vr` — 3 algebraic-law `@test`s pinning the
   error contract across the invalid-fd sweep.
3. `integration_test.vr` — 2 `@test`s composing the Result-shape
   with List iteration.
4. `regression_test.vr` — 3 `@test`s pinning the re-export
   resolution and Result-shape stability.

## 5. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Happy-path fsync round-trip | Requires tmpdir fixture. |
| 2 | `pread` / `pwrite` / `sync_directory` happy-path | Requires `core.io.fs` integration. |
