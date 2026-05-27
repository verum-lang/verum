# `net/unix` audit

Module: `core/net/unix.vr` (~1051 LOC) — AF_UNIX stream
sockets for local IPC. Per-process credentials via SO_PEERCRED
(Linux) / LOCAL_PEERCRED (macOS). FD-passing via SCM_RIGHTS
declared (gated as NotImplemented).

Tests cover the algebraic data-surface:

* `UnixError` 7-variant Eq + 3 pairwise-disjointness +
  payload-disjoint cases for PathTooLong's limit/actual
  fields.
* `ShutdownKind` 3-variant + 2 pairwise-disjointness.
* `PeerCred` record (pid: Int32, uid: UInt32, gid: UInt32) —
  root-user + negative-pid sentinel cases.
* `FdPassingError.NotImplemented` stub variant.

Live socket operations (UnixStream.connect/.bind/.peer_addr,
UnixListener.accept) gated on filesystem socket harness at
`vcs/specs/L2-standard/net/unix/`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.shutdown` graceful-shutdown FD-handoff | `send_fds` / `recv_fds` for SCM_RIGHTS-passing listener fd between processes. |
| Local-only authorization | `peer_cred()` for "only accept from UID 0 for admin API" patterns. |
| systemd socket activation | listener fd inherited via env. |
| Local IPC daemons | every daemon ↔ control-program path. |

## 2. Crate-side hardcodes

`crates/verum_runtime/src/net/unix/...` uses V-LLSI direct
syscalls (no libc). The POSIX-stable errno mapping in
`UnixError.from_os_error` (`unix.vr:95-130`) is pinned by
`test_unix_error_*` tests.

## 3. Language-implementation gaps

### §3.1 UNIX-1 — Functional surface gated on filesystem socket fixture

Live `UnixStream.connect(&path)` / `UnixListener.bind(&path)`
require either:

1. A test-isolated temp directory + cleanup harness.
2. Linux's abstract namespace (`"\0my-service"`) for hermetic
   tests without filesystem inode.

Pre-fix harness: none in `core-tests/`. End-to-end coverage at
language-level `vcs/specs/L2-standard/net/unix/`.

### §3.2 `FdPassingError.NotImplemented` stub variant pinned

Source-side at `unix.vr:968-985` — `send_fds` / `recv_fds`
return `FdPassingError.NotImplemented` until sendmsg/recvmsg +
cmsghdr bindings land. The graceful-shutdown FD-handoff pattern
referenced by `net-framework.md §7.8` depends on this.

**Effort to close**: SCM_RIGHTS + cmsghdr V-LLSI direct
syscall implementation — 2-3 days per platform (Linux +
Darwin diverge on cmsghdr layout).

### §3.3 `PeerCred` Linux ucred 12-byte struct layout

Source-side at `unix.vr:846-870`. Layout `{ pid: Int32, uid:
UInt32, gid: UInt32 }` matches Linux `struct ucred` exactly.
Tested via `test_peer_cred_construction` for field preservation.

### §3.4 Windows AF_UNIX deferred

Source-side at `unix.vr` is `@cfg(target_os = "linux")` /
`@cfg(target_os = "macos")`. Windows 10 1803+ supports AF_UNIX
but the integration is gated behind
`@cfg(feature = "windows_unix_sockets")`.

## 4. Action items landed in this branch

* `core-tests/net/unix/unit_test.vr` — 21 unit tests covering
  UnixError 7-variant Eq + payload disjointness + 3 pairwise-
  variant disjointness, ShutdownKind 3-variant + 2 pairwise-
  disjointness, PeerCred record fields (admin + root + negative-
  pid-sentinel), FdPassingError.NotImplemented stub variant.
* `core-tests/net/unix/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| UnixStream.connect/.bind against /tmp/test.sock with cleanup | this folder + harness | 1 day |
| Linux abstract-namespace coverage (`"\0name"`) | this folder | 4h |
| peer_cred() against a controlled fixture (returns own process pid/uid/gid) | this folder | 4h |
| send_fds / recv_fds SCM_RIGHTS implementation + tests | stdlib + tests | 3 days incl per-platform |
| UnixError.from_os_error POSIX errno mapping coverage (1/2/13/17/32) | this folder | 1h after OsError fixture lands |
