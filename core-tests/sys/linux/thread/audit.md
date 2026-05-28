# `core.sys.linux.thread` — implementation audit

## Status: **partial** (CLONE_* constant surface complete; live clone(2) deferred)

* Linux clone(2) CLONE_* flag bits pinned — 14 single-bit flags
  (CLONE_VM/FS/FILES/SIGHAND/THREAD/SETTLS/SYSVSEM/PARENT_SETTID/
   CHILD_CLEARTID/CHILD_SETTID/NEWPID/NEWNS/NEWUSER/DETACHED) +
  SIGCHLD=17 + CLONE_THREAD_FLAGS composite.

## Action items landed

1. `unit_test.vr` — 16 `@test`s pinning canonical Linux CLONE_* values.
2. `property_test.vr` — 3 laws: every CLONE_* flag is a power of two;
   pairwise disjoint; CLONE_VM/FS/FILES/SIGHAND consecutive bits.
3. `regression_test.vr` — 4 `@test`s: CLONE_VM=0x100; CLONE_THREAD=0x10000;
   SIGCHLD=17 (Linux value, NOT Darwin's 20); CLONE_THREAD_FLAGS contains
   CLONE_THREAD bit.

## Deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Live clone(2) + clone3 exercise | Needs target_os=linux. |
| 2 | LinuxThreadError variant sweep | Needs error-suite. |
