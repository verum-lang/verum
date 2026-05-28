# `core.sys.linux.io` — implementation audit

## Status: **partial** (io_uring constant surface complete; live ring deferred)

* io_uring setup / enter / register flag constants pinned.
* Live IoUringDriver / EpollDriver bringup deferred to target_os=linux.

## Action items landed

1. `unit_test.vr` — 18 `@test`s pinning IORING_SETUP_* (8 setup flags),
   IORING_ENTER_* (4 enter flags), IORING_REGISTER_* (6 register opcodes).
2. `property_test.vr` — 4 laws: SETUP flags power-of-two + pairwise
   disjoint + consecutive bits 0..=7; ENTER flags power-of-two.
3. `regression_test.vr` — 4 `@test`s: SETUP_IOPOLL bit 0; SETUP_SQPOLL
   bit 1; REGISTER_BUFFERS opcode 0; REGISTER_FILES opcode 2.

## Deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Live io_uring ring bringup | Needs target_os=linux kernel ≥ 5.1. |
| 2 | EpollDriver fallback test | Needs target_os=linux. |
| 3 | Operation opcode (IORING_OP_READ/WRITE/ACCEPT/etc.) sweep | Not yet conformance-tested. |
