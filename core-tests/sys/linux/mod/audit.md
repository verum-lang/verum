# `core.sys.linux.mod` — implementation audit

## Status: **partial** (constant umbrella surface; live FFI deferred)

* The Linux umbrella aggregates: errno, syscall, arch, auxv, mem, io,
  thread, time, tls, bpf.
* Conformance suite pins value-equivalence across the umbrella ↔
  direct-submodule paths for the most-touched constants.

## Action items landed

1. `unit_test.vr` — 7 `@test`s pinning umbrella reachability of errno
   (EAGAIN=11, ENOENT=2), mem (MAP_ANON=0x20, PROT_READ=1), time
   (CLOCK_MONOTONIC=1), tls (MAX_CONTEXT_SLOTS=256,
   STACK_GUARD_MAGIC).

## Deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Live syscall6 exercise | Needs target_os=linux. |
| 2 | bpf module conformance | BPF Map/Program loader requires live BPF subsystem. |
| 3 | Full umbrella path-equivalence sweep | First-pass covers core subset; expand as new submodules surface. |
