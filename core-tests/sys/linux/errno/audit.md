# `core.sys.linux.errno` — implementation audit

## Status: **complete** (under `--interp`; constant + predicate surface)

* This module ships POSIX/Linux errno constants and 4 classifier
  predicates. Reference source: `/usr/include/asm-generic/errno-base.h`
  + `/usr/include/asm-generic/errno.h`.
* Linux EAGAIN = 11 (DIVERGES from Darwin's 35) — the most common
  source of cross-platform errno bugs.

## Action items landed

1. `unit_test.vr` — 25 `@test`s pinning canonical Linux errno values
   (EPERM=1 through EPIPE=32 sample) + predicate behaviour
   (is_retryable / is_permission_error / is_not_found over canonical
   inputs + non-matching counter-examples).
2. `property_test.vr` — 8 algebraic laws: 30-element Linux low-POSIX
   subset pairwise distinct + consecutive 1..=30 sequence;
   EWOULDBLOCK ≡ EAGAIN ABI alias; EAGAIN=11 (Linux-divergent from
   Darwin's 35); predicates exhaustive over canonical subsets +
   total over Int.
3. `regression_test.vr` — 4 `@test`s pinning Linux-specific values
   that differ from Darwin (EAGAIN=11 NOT 35; EWOULDBLOCK ABI alias);
   is_retryable canonical set; EPERM/EACCES distinct codes.

## Deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Full errno_name / errno_short_name sweep | 100+ arm match table — exhaustive verification deferred. |
| 2 | Cross-platform errno divergence catalogue | Pin every Linux ↔ Darwin difference at a higher layer (core.sys.common). |
