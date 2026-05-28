# `core.sys.darwin.tls` — implementation audit

## Status: **partial** (constant + error-type surface complete; live TLS deferred)

* This module ships the macOS TLS bootstrap + V-LLSI context system
  implementation, including the per-thread TCB allocator.
* Conformance suite covers: 5 module constants (MAX_CONTEXT_SLOTS=256,
  CONTEXT_STACK_DEPTH=8, TCB_MAGIC, DEFAULT_STACK_SIZE=1MB,
  GUARD_PAGE_SIZE=4KB) + 8-variant SysTlsError ADT.
* Live TLS exercises (`init_main_thread_tls`, `ctx_get/set`, etc.)
  require an initialized TCB — VCS specs domain.

## Action items landed

1. `unit_test.vr` — 17 `@test`s: 5 constant pins (including the exact
   TCB_MAGIC ASCII pattern); 8 SysTlsError variant constructions (3
   unit + 5 payload); Eq reflexivity + payload sensitivity + variant
   disjointness; is_retryable always-false over the 6 representative
   error samples.
2. `property_test.vr` — 7 algebraic laws: MAX_CONTEXT_SLOTS +
   CONTEXT_STACK_DEPTH + GUARD_PAGE_SIZE power-of-two;
   DEFAULT_STACK_SIZE page-aligned + ≥ 64 KiB; Eq reflexive over unit
   + payload variants; Eq payload-sensitive; cross-variant disjointness
   even with identical payload codes.
3. `regression_test.vr` — 4 `@test`s: TCB_MAGIC exact value (defect-
   class pin); MAX_CONTEXT_SLOTS cross-platform consistency;
   CONTEXT_STACK_DEPTH matches the array-literal layout in
   DarwinContextEntry; TLS errors never retryable.

## Deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Live `init_main_thread_tls` round-trip | Needs uninitialized thread context — VCS specs domain. |
| 2 | `ctx_push` / `ctx_pop` stack overflow + underflow | Needs controlled context-stack fixture. |
| 3 | `pthread_key_create` / `pthread_setspecific` integration | FFI exercise gated on the libsystem live binding work. |
| 4 | TCB invariant scanning | Needs initialized thread + corruption injection. |
