# `core.sys.linux.mem` — implementation audit

## Status: **complete** (under `--interp`; PROT_*/MAP_* constant surface)

* Linux-specific mmap-family constants pinned. PROT_* universal POSIX
  values (1/2/4) shared across platforms; MAP_* Linux-specific (e.g.
  MAP_ANON = 0x20 vs Darwin's 0x1000).

## Action items landed

1. `unit_test.vr` — 17 `@test`s pinning PROT_NONE/READ/WRITE/EXEC +
   MAP_SHARED/PRIVATE/FIXED/ANON/ANONYMOUS + MAP_GROWSDOWN/STACK/HUGETLB/
   LOCKED/NORESERVE/POPULATE/NONBLOCK canonical Linux values.
2. `property_test.vr` — 6 algebraic laws: PROT_* power-of-two +
   pairwise disjoint + OR-combines-to-7; MAP_ANONYMOUS ≡ MAP_ANON
   alias; MAP_SHARED ⊕ MAP_PRIVATE disjointness; MAP_* pairwise
   distinct; MAP_ANON divergence from Darwin.
3. `regression_test.vr` — 3 `@test`s: MAP_ANON Linux-divergent;
   MAP_ANONYMOUS alias; PROT_* universal values.

## Deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Live mmap exercise | Needs target_os=linux build. |
| 2 | MAP_HUGE_2MB / MAP_HUGE_1GB explicit-page-size flags | Not yet conformance-pinned. |
