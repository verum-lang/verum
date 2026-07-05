# `intrinsics/lowlevel` audit

Modules: `core/intrinsics/lowlevel/{aarch64,x86_64,kernel,mmio}.vr`
(28 + 32 + 10 + 7 intrinsics) — architecture-specific register/instruction
intrinsics, kernel-mode ops, and memory-mapped-I/O accessors.

## Coverage decision: AUDIT-ONLY (no `*_test.vr`)

* `x86_64.vr` — cannot execute on the aarch64 test host (wrong ISA).
* `aarch64.vr` — reads privileged/system registers and issues barriers
  (`dmb`/`dsb`/`isb`); most are unsafe to invoke from a hosted test
  process and several fault outside kernel mode.
* `kernel.vr` — ring-0 operations (page-table, interrupt, CPU control);
  a user-mode test process cannot execute them.
* `mmio.vr` — `volatile_load`/`volatile_store` over `*volatile T` require
  a real MMIO region; there is no portable safe MMIO address to test.

These are the bare-metal / Tier-1 substrate; their correctness is a
codegen concern (LLVM inline-asm emission keyed off `module.get_triple()`)
verified by the no-libc `otool`/`ldd`/`dumpbin` procedure and the
architecture-specific codegen unit tests in `crates/verum_codegen`, not by
a hosted `.vr` suite.

## Contract notes

* `volatile_load`/`volatile_store` (mmio.vr) carry the canonical
  `*volatile T` / `*volatile mut T` pointer kind so the type checker
  enforces MMIO discipline at the call site (this replaced the stale
  duplicate `*const`/`*mut` declarations that used to shadow them — see
  `core/intrinsics/memory.vr` §6).

## Action items

* Per-arch codegen conformance lives in `crates/verum_codegen` tests +
  the no-libc verification procedure (`docs/architecture/no-libc-architecture.md`).
* A QEMU-gated bare-metal lane could exercise these end-to-end; deferred.
