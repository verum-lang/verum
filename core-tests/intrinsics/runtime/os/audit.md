# `intrinsics/runtime/os` audit

Module: `core/intrinsics/runtime/os.vr` (~386 LOC) — centralised
kernel-boundary intrinsics: file I/O, TCP/UDP, time, mmap, process,
context, defer, argv, concurrency.  Migrated from `core/sys/raw.vr` (#61).

Tests: unit (9) + integration (2) + property (9, added 2026-07-15) over
the FILE I/O subset — the surface that is safe to exercise from a test
process without a network or a live child.  Text-path convenience forms
(write_string/read_to_string/delete/open/close/size/mkdir) in unit;
raw-fd read/write over cbgr buffers + seek in integration; LAWS in
property: write→read round-trip identity over a UTF-8 boundary domain,
last-write-wins on rewrite, size-counts-BYTES (6 for "мир"), seek
algebra (SET returns target, CUR 0 reads it back, END 0 == size, SET 0
rewinds), delete lifecycle (open-after-delete fails; delete-missing
reports an error).

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
