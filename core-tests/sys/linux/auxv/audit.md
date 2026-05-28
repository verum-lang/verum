# `core.sys.linux.auxv` — implementation audit

## Status: **complete** (constant surface)

* AT_* auxiliary vector constants — 15 canonical ELF ABI values
  pinned (AT_NULL=0 terminator through AT_SYSINFO_EHDR=33 vDSO base).

## Action items landed

1. `unit_test.vr` — 15 `@test`s pinning AT_* values per Linux ABI.
2. `property_test.vr` — 3 laws: pairwise distinctness; AT_NULL first;
   UID/EUID/GID/EGID quartet consecutive (11/12/13/14).
3. `regression_test.vr` — 4 `@test`s pinning AT_NULL=0 (terminator);
   AT_PAGESZ=6 (page-size discovery); AT_RANDOM=25 (stack-canary seed);
   AT_SYSINFO_EHDR=33 (vDSO base).

## Deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Live auxv read on Linux | Needs target_os=linux build + ELF stack-init fixture. |
| 2 | AuxvEntry record round-trip | Type exists; not yet conformance-pinned. |
