# `intrinsics/runtime/os` audit

Module: `core/intrinsics/runtime/os.vr` (~386 LOC) — centralised
kernel-boundary intrinsics: file I/O, TCP/UDP, time, mmap, process,
context, defer, argv, concurrency.  Migrated from `core/sys/raw.vr` (#61).

Tests: unit (9) + integration (2) over the FILE I/O subset — the surface
that is safe to exercise from a test process without a network or a live
child.  Text-path convenience forms (write_string/read_to_string/delete/
open/close/size/mkdir) in unit; raw-fd read/write over cbgr buffers +
seek in integration.

## Coverage decisions

* **Networking** (`__tcp_connect_raw`, `__tcp_listen_raw`, UDP) needs a
  live peer/port — covered by `core-tests/net/*`, not duplicated here.
* **Process spawn / argv / context / defer** intrinsics mutate global or
  spawn children — out of scope for a value-level suite; language-level
  coverage lives in `vcs/specs/L2-standard`.
* **mmap** (`__sys_mmap_raw`) is exercised by the CBGR allocator suites
  (`core-tests/mem/*`).

## Contract notes (pinned)

* `file_open` flags: 0=read, 1=write, 2=rw, 0x100=create, 0x200=truncate,
  0x400=append (composable); returns fd ≥ 0 or a negative error.
* `file_write`/`file_read` over raw Int buffers return byte counts;
  `file_seek` whence 0=SET/1=CUR/2=END returns the new offset.
* `file_delete`/`mkdir` return 0 on success, negative on error.
* Text-path convenience forms (`file_write_string`/`file_read_to_string`)
  create/truncate and marshal the Text payload without the raw-pointer
  round-trip.

## Crate-side drift surfaces

* Two-tier dispatch: interp handlers in
  `handlers/{io_engine,calls}.rs` (name-dispatched `__*_raw` keys) ↔ AOT
  per-triple syscall lowering (`verum_codegen/llvm/*`), keyed off
  `module.get_triple()` — never host `#[cfg]` (no-libc invariant).

## Action items

**Landed**
* File-I/O conformance suite (Text-path + raw-fd-over-cbgr).
* OS-FILEOPEN-FLAG-DRIFT-1 (task #6) — PARTIALLY FIXED both tiers:
  - interp `file_open` accepts the documented abstract O_CREAT bit (0x100)
    alongside the Linux 0x40.
  - AOT `emit_file_open` REWRITTEN from a 3-way mode-selector (which
    misread an abstract flag word like 0x301 as read-mode) to abstract-flag
    decoding: access = flags & 0x3 (0/1/2 map 1:1 to O_RDONLY/WRONLY/RDWR),
    then OR in the platform O_CREAT/O_TRUNC/O_APPEND bits (per target
    triple) for the 0x100/0x200/0x400 abstract bits.  Validated both tiers
    by `regression_open_create_flag_creates_file`.

## AOT residual — OS-AOT-SURFACE gap (interp-green, AOT-incomplete)

The os conformance suite is GREEN on the interpreter (11/11).  On AOT only
the create-flag regression passes (2/11) — the broader AOT file-I/O
intrinsic surface is incomplete: the Text-path helpers
(`__file_write_string_raw`/`__file_read_to_string_raw`), fd lifecycle
(`__file_open_raw` for read/`__file_close_raw`/`__file_size_raw`),
`__file_delete_raw`, `__mkdir_raw`, and the raw-fd byte read/write
(`__file_read_raw`/`__file_write_raw` — the latter two have NO AOT
buffer-address lowering) do not round-trip under AOT.  Tracked as a
documented AOT residual (the project convention for interp-complete /
AOT-partial suites — cf. atomic 25/30, conversion 52/60), NOT @ignored, so
the interp coverage stays live.  The AOT os-intrinsic surface is a
dedicated follow-up under task #6.
* OS-RAWFD-BUF-STUB-1 FIXED (2026-07-05): `__file_read_raw`/`__file_write_raw`
  were STUBS that ignored the buffer address — read filled a discarded local
  vec (bytes vanished), write pretended (returned `len` untouched).  The
  stubs predated honest raw addressing; a cbgr / mem_raw buffer is now a
  real dereferenceable address (proven by the mem_raw suite), so both now
  copy through it.  Same inert-stub class as MEMRAW-CANONICAL-NAMES-INERT-1.
* file_open interp handler widened to accept the documented abstract
  O_CREAT bit (0x100) alongside Linux 0x40 — partial OS-FILEOPEN-FLAG-DRIFT-1
  (task #6).

**Deferred**
* Network fd round-trip → `core-tests/net`.
* argv / process-spawn → language-level suite.
