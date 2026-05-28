# `core.sys.linux.syscall` — implementation audit

## Status: **partial** (architecture-invariant ABI constants pinned; SYS_* numbers + raw syscall6 deferred)

* Architecture-invariant ABI constants pinned: termios ioctls
  (TCGETS/TCSETS/TCSETSW/TCSETSF/TIOCGWINSZ/TIOCSWINSZ), clone/fork
  flags (FORK_FLAGS=17, CLONE_VFORK_FLAGS composite), fcntl ops
  (F_GETFD/SETFD/GETFL/SETFL + FD_CLOEXEC + O_NONBLOCK=0o4000),
  wait flags (WNOHANG/WUNTRACED/WCONTINUED), futex ops
  (FUTEX_WAIT/WAKE/WAIT_PRIVATE/WAKE_PRIVATE).
* SYS_READ / SYS_WRITE / SYS_CLOSE / etc. are
  `@cfg(target_arch="x86_64")` / `@cfg(target_arch="aarch64")`
  gated — only reachable in the active arch build. Deferred to
  per-arch conformance.

## Action items landed

1. `unit_test.vr` — 23 `@test`s: termios ioctls (6); clone/fork
   flags (2); fcntl ops (6); wait flags (3); futex ops (4).
2. `property_test.vr` — 5 algebraic laws: fcntl ops consecutive
   1..=4; wait flags power-of-two + pairwise disjoint; futex
   private-flag consistency; futex ops distinct.
3. `regression_test.vr` — 5 `@test`s: TIOCGWINSZ=0x5413;
   FD_CLOEXEC=1; O_NONBLOCK octal 0o4000; FUTEX_WAIT_PRIVATE=128;
   FORK_FLAGS=17.

## Deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | SYS_* syscall numbers per-arch | x86_64 + aarch64 sweeps need separate suites under linux/arch/. |
| 2 | Live syscall_raw / syscall6 round-trip | Needs target_os=linux build. |
| 3 | F_OK + remaining fcntl constants | Some still in scope; second-pass coverage. |
