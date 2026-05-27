# `net/shutdown` audit

Module: `core/net/shutdown.vr` (~323 LOC) — Graceful-shutdown
primitives for any accept-loop service:

* `GracefulShutdown` controller — owns CancellationTokenSource +
  AtomicU64 in-flight counter + initiation Instant.
* `ConnectionGuard` — RAII counter (fetch_add on acquire,
  fetch_sub on drop via Drop impl).
* `tcp_listener_from_raw_fd` / `unix_listener_from_raw_fd` —
  adopt a listener fd (zero-downtime restart).
* `listen_fds` / `listen_fd` — systemd socket-activation
  protocol.

Tests cover the algebraic data-surface:

* `ShutdownError.DrainTimeout { remaining }` Eq + payload
  preservation.
* `ListenFdsError` 3-variant (NoSocketActivation / WrongPid /
  ParseError) + disjointness.
* `GracefulShutdown.new()` controller construction smoke test.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.weft` accept loop | shutdown.token() + shutdown.track() per request. |
| `core.net.tcp` / `core.net.unix` | listener_from_raw_fd for zero-downtime restart. |
| `core.action.exec` orchestrators | shutdown.shutdown(timeout) one-call form. |
| systemd-service launcher | listen_fds() protocol. |

## 2. Crate-side hardcodes

The systemd socket-activation env-var protocol (`LISTEN_FDS`,
`LISTEN_PID`, FD-3 start) is pinned in the implementation but
not exposed as `public const`. Out-of-band drift would break
systemd integration silently.

## 3. Language-implementation gaps

### §3.1 SHUTDOWN-1 — `GracefulShutdown.initiate` /
       `.wait_drained` / `.shutdown` runtime paths

Subject to precompile-cascade SIGSEGV class (multi-threaded
counter + cancellation token + mutex-guarded Instant Maybe).
Tested at L2 specs end-to-end (`vcs/specs/L2-standard/net/shutdown/`).

### §3.2 FD-handoff via SCM_RIGHTS (gated on `core.net.unix.send_fds`)

The graceful-shutdown FD-handoff pattern referenced by
net-framework.md §7.8 depends on `core.net.unix.send_fds` /
`recv_fds` — currently `FdPassingError.NotImplemented`. See
`net/unix/audit.md §3.2`.

### §3.3 systemd socket-activation env-var protocol

`listen_fds` reads `LISTEN_FDS` + `LISTEN_PID` per the systemd
spec. WrongPid sentinel covered by enum data-shape.

## 4. Action items landed in this branch

* `core-tests/net/shutdown/unit_test.vr` — 8 unit tests
  covering ShutdownError DrainTimeout Eq + payload disjoint +
  zero-value, ListenFdsError 3-variant construction + 1
  pairwise-disjoint, GracefulShutdown.new smoke test.
* `core-tests/net/shutdown/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| GracefulShutdown.token / .track / .initiate / .wait_drained / .shutdown end-to-end | this folder + harness | 4h, gated on multi-threaded harness |
| ConnectionGuard Drop semantics (refcount restore) | this folder | 2h |
| listen_fd(i) against fixture LISTEN_FDS env-var | this folder + env-mutation harness | 4h |
| listener_from_raw_fd round-trip with TcpListener.bind | this folder | 4h after fd-adoption fixture |
| tcp_listener_from_raw_fd vs unix_listener_from_raw_fd disambiguation | this folder | 1h |
| Property: ∀guard. drop_then_count == count-1 | this folder | 2h |
