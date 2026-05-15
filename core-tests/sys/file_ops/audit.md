# `core.sys.file_ops` — implementation audit

## Status: **partial** (raw-intrinsic mount migration landed; full I/O round-trip deferred)

* Every public function on the wrapper surface (`read_file` /
  `write_file` / `append_file` / `file_exists` / `file_size` /
  `delete_file`) is covered by `unit_test.vr` at the error-sentinel
  layer (missing path → None / -1 / false).
* Every `OpenMode` constructor (`read` / `write` / `read_write` /
  `append` / `create`) has its POSIX-canonical flag bit-pattern pinned
  in `unit_test.vr` and the bit-set invariants (O_CREAT presence,
  O_APPEND presence, etc.) are pinned in `property_test.vr`.
* `integration_test.vr` composes the error sentinels with `Maybe` and
  `List` patterns user code reaches for.
* The **happy-path round-trip surface** (`write_file` → `read_file` of
  the same content; `append_file` accumulating; `file_size` reading
  the byte length after write) is **deferred** — it requires a
  tmpdir fixture and a guarantee that the test runner allows the
  interpreter to actually perform filesystem mutations. Tier-1 AOT
  takes the same `__file_*_raw` intrinsics through the FFI bridge,
  so the architectural contract is unchanged; we just need the
  fixture infrastructure to run them.

## 1. Cross-stdlib usage

`core.sys.file_ops` is the thin Verum-side shim over the raw FFI
surface for the four canonical POSIX file-IO syscalls. Consumers:

| Consumer | Touches | Notes |
|---|---|---|
| `core/io/fs.vr` | `read_file` / `write_file` | The higher-level `core.io.fs::File` wraps these and adds CBGR-typed handles. |
| `core/sys/durability.vr` | `__file_close_raw` (via fsync flow) | crash-safe persistence shim. |
| `core/sys/process_ops.vr::Child` | `__fd_close_raw` | child stdout/stderr cleanup. |

## 2. Crate-side hardcodes

| Site | What it pins | Status |
|---|---|---|
| `crates/verum_vbc/src/interpreter/dispatch_table/handlers/calls.rs` | `__file_*_raw` intrinsic dispatch table | OK (file ops go through `std::fs`) |
| `crates/verum_codegen/src/llvm/ffi.rs` | LLVM lowering of the same intrinsics for AOT | OK |

## 3. Language-implementation gaps surfaced by this suite

### 3.1 Stale `super.raw.*` mount (CLOSED in this branch)

* **Symptom (pre-fix)**: `mount super.raw.*` at `core/sys/file_ops.vr:18`
  pointed at a `core/sys/raw.vr` file that was migrated away to
  `core/intrinsics/runtime/os.vr`. Every `__file_*_raw` call failed
  function-id lookup at codegen time and the wrappers compiled to
  lenient panic-stubs.
* **Architectural class**: Same as the one already closed for
  `time_ops.vr`. Sister files closed in the same branch: `context_ops.vr`,
  `process_ops.vr`, `net_ops.vr`.
* **Status**: **CLOSED** in this branch by replacing
  `mount super.raw.*` with `mount core.intrinsics.runtime.os.{...}`.
  Pinned by `regression_test.vr` §A.

### 3.2 Selective re-export resolution (already closed at the parent layer)

* **Status**: **closed** — see `core-tests/sys/common/audit.md` §3.1.

## 4. Action items landed in this branch

1. **Fundamental fix**: replaced `mount super.raw.*` in
   `core/sys/file_ops.vr` with the canonical
   `mount core.intrinsics.runtime.os.{__file_*_raw}`.
2. `unit_test.vr` — 13 `@test`s covering OpenMode bit patterns +
   error-sentinel paths for the I/O wrappers.
3. `property_test.vr` — 8 algebraic-law `@test`s pinning O_*
   bit-flag invariants and missing-path sentinel partitioning.
4. `integration_test.vr` — 3 `@test`s composing the error sentinels
   with Maybe and List patterns.
5. `regression_test.vr` — 6 `@test`s pinning the closed
   stale-super-raw mount defect (§A) and the OpenMode dispatch
   stability (§B).

## 5. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Happy-path write→read round-trip | Requires tmpdir fixture; should land alongside `core-tests/io/fs/` integration tests. |
| 2 | `append_file` body misroutes via `__file_write_string_raw` for the second write | Pre-existing stdlib bug: `append_file` opens a fd with O_APPEND flags but then writes via `__file_write_string_raw(path, content)` (path-based) instead of `__file_write_raw(fd, ...)`. The fd-based append is required to honour `O_APPEND` semantics. Tracked as `core.sys.file_ops.append_file`. |
