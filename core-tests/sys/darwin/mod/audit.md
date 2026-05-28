# `core.sys.darwin.mod` — implementation audit

## Status: **complete** (under `--interp`; umbrella re-export surface)

* This module is the macOS-specific umbrella namespace — re-exports
  the union of libsystem, mach, errno, tls, io, thread, time public
  surfaces.
* The conformance suite verifies the umbrella RE-EXPORT contract:
  every type / constant / function reachable through
  `mount core.sys.darwin.X;` is byte-identical to the same name reached
  via the direct submodule path.

## 1. Cross-stdlib usage

`core.sys.darwin.mod` is referenced from `core.sys.mod` via
`@cfg(target_os = "macos") public module darwin;`.

| Caller | Path |
|---|---|
| `core.sys` | platform-conditional re-export gated by target_os. |
| User code | rarely — most users mount `core.sys.X` (which routes to the platform-conditional submodule). |
| Runtime authors | direct `mount core.sys.darwin.X` for macOS-specific operations. |

## 2. Action items landed

1. `unit_test.vr` — 23 `@test`s pinning umbrella reachability across
   constants from each submodule: libsystem (O_*, PROT_*, MAP_*, SEEK_*,
   CLOCK_*); mach (KERN_SUCCESS, VM_PROT_*, SYNC_POLICY_*); errno
   (EPERM/ENOENT/EACCES/EAGAIN/EINTR + classifier predicates); tls
   (MAX_CONTEXT_SLOTS, CONTEXT_STACK_DEPTH, TCB_MAGIC); io (MAX_EVENTS,
   DEFAULT_TIMEOUT_NS); thread (ONCE_*); time (CLOCK_*_ID).
2. `property_test.vr` — 11 path-equivalence laws verifying constants
   reached via umbrella exactly match the same names reached via direct
   submodule mount (libsystem.O_RDONLY / O_CREAT / MAP_ANON;
   mach.KERN_SUCCESS / VM_PROT_READ; errno.EAGAIN / EINTR;
   tls.MAX_CONTEXT_SLOTS; io.MAX_EVENTS; thread.ONCE_INIT;
   time.CLOCK_MONOTONIC_ID).
3. `integration_test.vr` — 7 cross-stdlib scenarios composing umbrella
   constants in real usage patterns: open(2) flag composition,
   mmap permission composition, errno classifier dispatch table,
   ONCE_* state machine, KERN_SUCCESS predicate, VM_PROT composition.
4. `regression_test.vr` — 5 `@test`s pinning every value-preservation
   contract for the umbrella mount chain (O_CREAT BSD-not-Linux,
   MAP_ANON BSD-value, EAGAIN=35, KERN_SUCCESS=0, TCB_MAGIC exact).

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Function re-exports validation | The umbrella also re-exports `safe_*` aliases (safe_mmap as mmap, etc.) — these need live FFI exercise; deferred to libsystem live-fixture work. |
| 2 | Type-identity preservation across umbrella + submodule | The currently-validated constants and the bench tests cover the contract; the structural-type re-export (KEvent / Stat / Sockaddr) needs further coverage. |
