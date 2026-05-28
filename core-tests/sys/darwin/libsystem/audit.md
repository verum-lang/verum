# `core.sys.darwin.libsystem` — implementation audit

## Status: **partial** (constants surface complete; FFI function surface needs fixtures)

* This module exposes the POSIX/BSD constant table that user code
  reaches for when mounting Darwin-side FFI — open flags, prot flags,
  mmap flags, madvise codes, kqueue filters, socket constants, clock
  IDs, signal numbers.
* The FFI function bindings (`read` / `write` / `mmap` / `kqueue` /
  `pthread_*` / `clock_gettime`) are exercised through their `safe_*`
  wrappers in `core/sys/darwin/libsystem.vr` — those need real fd
  fixtures and live socket pairs, deferred to per-platform integration.

## 1. Cross-stdlib usage

`core.sys.darwin.libsystem` is the macOS-specific equivalent of
`core.sys.linux.syscall` — every macOS file / mmap / socket / kqueue /
thread / clock operation routes through it.

## 2. Action items landed in this branch

1. `unit_test.vr` — 27 `@test`s pinning O_*, SEEK_*, PROT_*, MAP_*,
   MADV_*, CLOCK_* canonical BSD values.
2. `property_test.vr` — 9 algebraic laws: PROT_* power-of-two +
   pairwise-disjoint + OR-combines-to-7; SEEK_{SET,CUR,END} partition
   of {0,1,2}; open-flag OR-composition contracts; MADV_* identifier
   distinctness; MAP_SHARED ⊕ MAP_PRIVATE disjointness.
3. `integration_test.vr` — 8 cross-stdlib scenarios: canonical open-
   mode table (read_only / write_only / append / create_new) with
   O_CREAT classification; text/data/rwx segment permission patterns;
   custom SeekFrom ADT dispatch to libc-whence values; anonymous
   private + shared mmap flag composition; O_CLOEXEC propagation
   through fd-leak-safe open chain.
4. `regression_test.vr` — 4 `@test`s pinning BSD-specific values that
   diverge from Linux (O_CREAT=0x200 not 0x40; O_CLOEXEC=0x1000000 not
   0x80000; MAP_ANON=0x1000 not 0x20) + universal PROT_ values.

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | FFI function bindings live exercise | `safe_read` / `safe_write` / `safe_mmap` etc. need real fd + fixture pair. Deferred to per-platform integration suite. |
| 2 | kqueue event filter surface (EVFILT_*) | The 8+ kqueue filter constants are pinned in the source but not yet conformance-tested. Lives in `core-tests/sys/darwin/io/`. |
| 3 | Socket / signal / fcntl constant sweep | The remaining 50+ constants (AF_*, SOCK_*, SIG*, F_*) await per-area expansion. |
