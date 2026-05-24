# `core.io.fs` — audit

> Conformance suite for `core/io/fs.vr`.
> Snapshot: 2026-05-24.
> Tier 0 (`--interp`) status: **partial** — FileType (4 variants),
> Permissions (Unix mode wrapper) are stable. Filesystem operations
> (metadata / exists / read_dir / create_dir / rename / copy / etc.)
> are gated by task #io-1 + need for a temp-fixture harness (#io-8).

## 1. Cross-stdlib usage

* `core.io.file.File` — calls `metadata` to query file size.
* `core.io.path.Path.exists` / `Path.is_file` / `Path.is_dir` —
  thin wrappers around fs.metadata.
* `core.io.process.Command` — uses `current_dir` for relative path
  resolution.
* `core.cli.app` — `temp_dir` for scratch files.

## 2. Crate-side hardcodes

* Permissions Unix mode uses `PERM_MASK = 0o7777` — pinned in source
  at `core/io/fs.vr` (line near 405). No Verum-side drift.
* `crates/verum_vbc/src/intrinsics/registry.rs` registers
  `__fs_stat` / `__fs_readdir` / `__fs_mkdir` / etc. — these are
  bare-fn intrinsic refs, no data-shape drift.

## 3. Language-implementation gaps

### §A — fs operations gated by #io-1 + temp-dir harness

Every test against real filesystem state (exists, metadata, read_dir,
walk_dir, copy, rename) requires:
1. #io-1 close (so the syscall wrappers route through proper method dispatch)
2. A temp-fixture harness in core-tests/ (so tests get isolated dirs)

### §B — `FileType` has only 4 variants, NOT the 8 POSIX file types

The Verum FileType enum collapses POSIX's 8 file types (regular, dir,
symlink, block device, char device, fifo, socket, unknown) into 4
(File, Dir, Symlink, Unknown). This is a deliberate API simplification.

If consumers need block-device / char-device / fifo / socket
discrimination, they must drop to `as_raw_fd` + libc-level
`S_ISCHR(stat.st_mode)` etc. — not yet exposed.

**Tracking task #io-10**: decide if Verum should expose the full
POSIX 8-type set or keep the 4-type abstraction. Closing this is a
public-API decision, not a defect.

### §C — Windows Permissions surface is `@cfg(unix)` only

The Permissions type is unix-gated. Windows has no equivalent here —
consumers need to handle the absence at the call site, or the type
needs a Windows fallback (read-only flag from FileAttributes).

**Tracking task #io-11**: Permissions implementation for Windows.

## 4. Action items landed

* Created `core-tests/io/fs/` with `unit_test.vr` (FileType 4 variants
  + 6 is-predicates, Permissions mode round-trip), `property_test.vr`
  (predicate mutual exclusivity, mode preservation), `integration_test.vr`
  (FileType filtering, Permissions cloning), `regression_test.vr`
  (7 `@ignore`'d pins for live fs ops + 1 documentation).

## 5. Action items deferred

| Task | Title | Estimate |
|---|---|---|
| #io-1 | Mount-scope-aware lookup_function | 3-5 days |
| #io-8 | Temp-dir / fixture harness in core-tests | 1 day |
| #io-10 | Decide POSIX 8-type FileType expansion | API design |
| #io-11 | Permissions implementation for Windows | 1-2 days |
