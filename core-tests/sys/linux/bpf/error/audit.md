# `core.sys.linux.bpf.error` — implementation audit

## Status: **complete** (under `--interp`; ADT surface)

* Provides `BpfError` — the typed-error ADT that every bpf() syscall
  wrapper in `core.sys.linux.bpf.{program, map}` returns.  Defines 8
  variants covering the documented failure modes of bpf() — raw errno,
  verifier rejection (with verifier log), invalid attach-type / program-
  type pairing, feature-not-supported (BTF mismatch on older kernels),
  map-type mismatch, key/value size mismatch, not-found, and unsupported
  helper-function ID.
* Reference: `<linux/bpf.h>` UAPI + `man 2 bpf` + Linux kernel
  `kernel/bpf/syscall.c` verifier paths.

## 1. Cross-stdlib usage

| Caller | Use |
|---|---|
| `core.sys.linux.bpf.program` | `load_program` / `attach_*` return `Result<_, BpfError>`. |
| `core.sys.linux.bpf.map` | `create_map` / `map_lookup` / `map_update` / `map_delete` return `Result<_, BpfError>`. |
| user code | XDP / tracing programs catch `BpfError.VerifierRejected { log }` to surface the verifier log to the developer. |

## 2. Action items landed in this branch

1. `unit_test.vr` — 20+ `@test`s pinning all 8 variant constructors,
   payload field round-trip via match destructure, Display textual
   output for the surface-facing variants, and the Eq protocol over
   same-variant / cross-variant / payload-mismatch pairs.

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | property_test.vr — Eq is reflexive / symmetric / transitive over the variant set | Deferred until the broader stdlib base/protocols/Eq property suite ships a generic property runner. |
| 2 | regression_test.vr | No defects yet pinned; will fill as the kernel-facing surface is exercised on a Linux host. |
| 3 | integration_test.vr | Will compose `BpfError` with `Result<_, BpfError>` for cross-stdlib error funnels once `program` / `map` integration surface lands. |
