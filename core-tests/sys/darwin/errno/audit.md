# `core.sys.darwin.errno` — implementation audit

## Status: **complete** (under `--interp`; constant + predicate surface)

* This module ships ~110 BSD/POSIX errno constants for macOS plus 6
  classifier predicates (is_retryable / is_connection_error /
  is_permission_error / is_not_found / is_resource_exhausted /
  is_would_block) and 2 textual helpers (errno_name / errno_short_name).
* Reference source: `/usr/include/sys/errno.h`. Constants are stable
  across macOS versions per Darwin ABI commitment.

## 1. Cross-stdlib usage

`core.sys.darwin.errno` is the canonical errno-value source for every
macOS syscall wrapper that surfaces the `errno` thread-local. Consumers:

| Caller | Use |
|---|---|
| `core.sys.darwin.libsystem` | `set_errno` / `errno` accessors, predicate-based retry loops. |
| `core.sys.common` | OSError construction from raw errno; cross-platform funnel. |
| `core.sys.darwin.io` | kqueue error-stream classification. |
| `core.io.fs` | Error funnel from POSIX path-resolution failures. |

## 2. Action items landed in this branch

1. `unit_test.vr` — 38 `@test`s pinning canonical errno values (EPERM=1
   through ENOSYS=78) + predicate behaviour over the documented domain.
2. `property_test.vr` — 11 algebraic laws including the 34-element
   "low POSIX" subset (1..=34) being pairwise distinct + consecutive
   (every value in 1..=34 is reachable); EWOULDBLOCK ≡ EAGAIN BSD-alias;
   predicate-disjointness over a representative error sample; total-
   over-Int (no panic on any input value).
3. `integration_test.vr` — 6 cross-stdlib scenarios: List<Int>
   retry-worth classification; custom ErrorClass ADT dispatch table;
   Maybe<Int> syscall-return funnel; retry-loop coordinator with
   break-on-non-retryable; permission-error funnel via Result<Int, Text>.
4. `regression_test.vr` — 4 `@test`s pinning known defect classes:
   EAGAIN/EWOULDBLOCK alias; EAGAIN = Darwin-35 (NOT Linux-11); canonical
   TCP failure set covered by is_connection_error; EINTR classified as
   retryable.

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | `errno_name` / `errno_short_name` full sweep | 110-arm match table — exhaustive verification deferred until full-suite property tests can leverage compile-time match-coverage. |
| 2 | Cross-platform errno comparison sweep | Pin every errno constant against the Linux counterpart's value — the divergence catalogue belongs in `core-tests/sys/common/` once the common error layer ships its canonical numeric-to-name mapping. |
