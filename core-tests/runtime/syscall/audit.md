# `runtime/syscall` audit

Module: `core/runtime/syscall.vr` (10 LOC) — raw 6-argument syscall
intrinsic.  Single function `syscall6(nr, a1..a6) -> Int`, the bottom of
the no-libc execution stack.

Tests: 2 surface-pinning unit tests (symbol existence + arity);
deliberately no kernel-invoking tests in this folder — those live at
`vcs/specs/L2-standard/sys/` with `@expected-kernel-call` markers.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.sys.linux.*` | every public sys-call (read/write/open/close/mmap/munmap/futex/clone/...) lowers to `syscall6(nr_linux, args...)`. |
| `core.sys.darwin.*` | dispatches into libSystem.B.dylib via separate path; this intrinsic is **NOT** used on Darwin (architectural invariant per CLAUDE.md "no libc" rule). |
| `core.sys.freebsd.*` | direct syscall (mirrors Linux pattern). |
| `core.runtime.time.monotonic_nanos` | falls through to `clock_gettime` via syscall6 on Linux/FreeBSD. |
| `core.runtime.tls.tls_get_base` | `arch_prctl` / `mrs tpidr_el0` per arch. |

## 2. Crate-side hardcodes

| Site | What it pins | Risk |
|---|---|---|
| `crates/verum_codegen/src/llvm/syscalls.rs` (per-arch inline asm) | x86_64 → `syscall` instr; aarch64 → `svc #0`; arm → `svc #0` | Drift drops to libc indirect — violates no-libc invariant. |
| `crates/verum_codegen/src/llvm/target_triple.rs` | `target_is_linux` / `target_is_freebsd` gating | HOST gating instead of TARGET miscompiles cross builds. |
| Linux syscall number tables per arch | x86_64 numbers ≠ aarch64 numbers ≠ arm32 numbers | Cross-arch number drift silently calls the wrong syscall. |

## 3. Language-implementation gaps

### §A — `verum.runtime.syscall6` intrinsic registration

Same dispatch-binding question as cbgr-§A / sync-§A:

```
grep -rn "verum.runtime.syscall6" crates/
```

Returns empty.  Either the binding lives under a different ident (look
for `syscall6` directly in `crates/verum_codegen/src/llvm/syscalls.rs`)
or the surface is currently inert.  Audit deferred.

### §B — no live invocation tests in this folder

By design.  A live `syscall6` call requires either:
* A safe wrapper (e.g., `getpid` on Linux: `syscall6(39, 0, 0, 0, 0, 0, 0)`)
  — but arch-dependent.
* Mocking the syscall — defeats the point.

Live tests are deferred to `vcs/specs/L2-standard/sys/` where per-arch
`@cfg(target_arch = "x86_64")` gates can safely invoke `getpid` / `gettid`
and verify the result matches an expected shape.

## Action items landed in this branch

* `core-tests/runtime/syscall/unit_test.vr` — 2 surface-pinning unit
  tests (no kernel calls).
* `core-tests/runtime/syscall/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Per-arch getpid round-trip test | `vcs/specs/L2-standard/sys/` | 30 min per arch |
| Drift-pinning unit test on syscall number tables | `crates/verum_codegen/tests/` | 1 h |
| `verum.runtime.syscall6` dispatch binding audit | `crates/` | 1 h |
