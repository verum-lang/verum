# `intrinsics/runtime/syscall` audit

Module: `core/intrinsics/runtime/syscall.vr` (~67 LOC) — raw `syscall0..6`.

## Coverage decision: AUDIT-ONLY (no `*_test.vr`)

Raw syscalls take PLATFORM-SPECIFIC numbers (Linux x86_64 vs aarch64 vs
macOS's 0x2000000-class offsets).  A number that is `getpid` on one target
is a different — potentially destructive — call on another; a conformance
suite that hardcodes numbers is a portability landmine, and there is no
portable no-op syscall.  The SAFE portable surface over the kernel boundary
is `runtime/os.vr` (`__sys_getpid_raw`, file I/O, …) and `runtime/time.vr`
— both suite-covered.  Placeholder tests would be decoration (same
precedent as `context/layer`).

## Contract notes (pinned by inspection)

* Dispatch: `@vbc_direct_lowering` → `SyscallLinux (0x45)` under VBC;
  AOT emits `syscall` / `svc #0` per `module.get_triple()` — target-keyed,
  never host `#[cfg]` (no-libc invariant).
* Return: raw kernel value; errno-style negatives are NOT translated at
  this layer (that is `core/sys`'s job).

## Action items

* Language-level differential tests (Linux-gated `getpid` round-trip vs
  `os.__sys_getpid_raw`) belong in `vcs/specs/L2-standard` with
  `@cfg(target_os)` gating — deferred until the vtest cfg-gate is wired.
