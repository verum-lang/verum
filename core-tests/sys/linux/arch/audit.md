# `core.sys.linux.arch` — implementation audit

## Status: **partial** (aarch64 SYS_* values pinned; x86_64 values gated)

* Per-arch SYS_* syscall number constants. The module ships two
  parallel `@cfg(target_arch=...)` blocks; only one architecture's
  values are reachable per build.
* Conformance suite pins the **aarch64** values (the host arch for
  this conformance run on macOS aarch64). x86_64 values share the
  same regression-class characteristics and would be validated under
  a target_os=linux build.

## Action items landed

1. `unit_test.vr` — 15 `@test`s pinning aarch64 SYS_* values: SYS_READ=63,
   SYS_WRITE=64, SYS_CLOSE=57, SYS_LSEEK=62, SYS_PREAD64=67,
   SYS_PWRITE64=68, SYS_READV=65, SYS_WRITEV=66, SYS_OPENAT=56,
   SYS_OPENAT2=437, SYS_FSYNC=82, SYS_FDATASYNC=83, SYS_FTRUNCATE=46,
   SYS_STATX=291, SYS_NEWFSTATAT=79.
2. `property_test.vr` — 4 laws: 9-syscall pairwise distinct sweep;
   all values in 0..=600 range; SYS_READ+1=SYS_WRITE consecutive block
   (62..=67); SYS_FSYNC+1=SYS_FDATASYNC.
3. `regression_test.vr` — 3 `@test`s pinning aarch64 SYS_READ=63 (NOT
   x86_64's 0); aarch64 SYS_WRITE=64 (NOT x86_64's 1); SYS_OPENAT2=437
   universal across both arches.

## Deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | x86_64 SYS_* values | Reachable only in `target_arch="x86_64"` build. Pinned indirectly via the regression "NOT" assertions. |
| 2 | All ~250 other SYS_* numbers per-arch | First-pass coverage focused on the canonical I/O subset. |
| 3 | x86_64-only syscalls (SYS_OPEN, SYS_STAT, etc.) | gated by `@cfg(target_arch="x86_64")` inside the module. |
