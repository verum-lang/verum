# `intrinsics/runtime/mod` audit

Module: `core/intrinsics/runtime/mod.vr` (~81 LOC) — the runtime-intrinsics
umbrella: pure WILDCARD re-exports over twelve submodules (async_ops, tls,
syscall, cbgr, time, sync, text, tier, os, io, mem_raw, scripting); no
explicit lists.

Suite added 2026-07-15: unit (3) — the umbrella compile path + child-module
round-trips through fully-qualified mounts.

## 0. Why the suite mounts children fully-qualified

UMBRELLA-REEXPORT-RESOLVE-1 (task #21, see `../../mod/audit.md`): wildcard
propagation through `mount core.intrinsics.runtime.{name}` rolls per-run
map-walk dice — observed unbound: `cbgr_allocate`, `num_cpus`, `get_tier`,
`char_is_whitespace`, `__sys_getpid_raw`, `__io_engine_new_raw`,
`script_engine_new`, while SAME-MODULE siblings (`monotonic_nanos`,
`__io_engine_destroy_raw`, `script_engine_free`) resolved; plus one MIS-BIND:
`memory_fence` bound to a 1-arg fence (cross-module shadow with the atomic
family).  With no explicit lists in this umbrella, NO brace-mounted name is
stably pinnable — the suite pins the umbrella's COMPILE path (the historic
AOT-INTRINSIC-QUALIFIED-NAME-1 SIGABRT surface) and end-to-end child
behaviour through qualified mounts, which are deterministic.

The brace-mount acceptance block (12 names + 6 tests) is the commented
promotion target in unit_test.vr.

## 1. Contract notes

* The umbrella exists so `mount core.intrinsics.runtime.*` consumers see
  one flat namespace; the #21 fix must make that namespace DETERMINISTIC
  (ranked collision policy), not merely populated.
* `memory_fence` vs the atomic fence family is a REAL collision the ranked
  policy must decide and document (sync.memory_fence is 0-arg by contract).

## 2. Action items

* Task #21 — promote the acceptance block once the ranked function index
  lands.
* Consider EXPLICIT re-export lists here (mirroring `core/intrinsics/mod.vr`'s
  nested lists) as the determinism-by-construction alternative for the
  high-traffic names.
