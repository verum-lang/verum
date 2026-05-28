# `core.sys.linux.tls` — implementation audit

## Status: **partial** (constant surface complete; live TCB deferred)

* TLS bootstrap constants pinned: arch_prctl op codes (ARCH_SET_GS=0x1001
  / SET_FS=0x1002 / GET_FS=0x1003 / GET_GS=0x1004); TLS layout
  (TLS_ALIGN=64, DEFAULT_TLS_SIZE=4096, TLS_CTX_OFFSET=0,
  STACK_GUARD_MAGIC=0xDEAD_BEEF_CAFE_BABE); context-slot caps
  (MAX_CONTEXT_SLOTS=256, CONTEXT_STACK_DEPTH=8 — cross-platform with
  Darwin).

## Action items landed

1. `unit_test.vr` — 10 `@test`s pinning arch_prctl op codes,
   TLS layout, context-slot caps.
2. `property_test.vr` — 7 algebraic laws: arch_prctl ops form
   consecutive 0x1001..0x1004 block; TLS_ALIGN power-of-two;
   DEFAULT_TLS_SIZE page-aligned; MAX_CONTEXT_SLOTS / STACK_DEPTH
   power-of-two + match Darwin.
3. `regression_test.vr` — 2 `@test`s: STACK_GUARD_MAGIC canonical
   value; MAX_CONTEXT_SLOTS cross-platform invariant.

## Deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Live TCB allocation + bootstrap | Needs target_os=linux. |
| 2 | ThreadLocalError variant sweep | Type exists; needs separate suite. |
