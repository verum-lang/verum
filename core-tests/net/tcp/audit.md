# `net/tcp` audit

Module: `core/net/tcp.vr` (~1637 LOC) — TCP streams +
listeners + accept loop + AsyncRead/AsyncWrite + cancellation-
aware variants. Built directly on V-LLSI syscalls (no libc).

Tests cover the algebraic data-surface visible from a USER
test module — currently only the `Shutdown` 3-variant
(SHUT_RD / SHUT_WR / SHUT_RDWR) and its pairwise disjointness.

Functional surfaces (TcpStream.connect, TcpListener.bind /
.accept, async variants, set_nodelay / set_keepalive /
set_linger socket options) require a runtime socket binding —
loopback connect or a UNIX socketpair fixture — which is gated
on `vcs/specs/L2-standard/net/tcp/` end-to-end harness.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.weft` accept loop | `TcpListener.accept_async` + `incoming_async`. |
| `core.net.http` clients | `TcpStream.connect_async`. |
| `core.net.tls` | underlying transport for `TlsStream`. |
| `core.net.websocket` | post-Upgrade transport. |
| `core.net.proxy` CONNECT tunnel | TcpStream→TcpStream relay. |

## 2. Crate-side hardcodes

`crates/verum_runtime/src/net/tcp/...` uses V-LLSI direct
syscalls (no libc); `crates/verum_codegen/src/llvm/...` emits
per-platform sockaddr layouts. The on-wire 4-byte / 16-byte
`sockaddr_in` / `sockaddr_in6` layouts are pinned indirectly
via the `core.net.addr` Ipv4Addr.to_u32 / Ipv6Addr.octets
tests.

## 3. Language-implementation gaps

### §3.1 TCP-1 — Functional surface gated on socket fixture

Pre-fix harness: none in `core-tests/`. End-to-end coverage
of TcpStream.connect/.accept/.read_async/.write_async is at
language-level `vcs/specs/L2-standard/net/tcp/`. Data-shape
algebra is verified in this folder.

### §3.2 `Shutdown.Read` / `.Write` / `.Both` translate to SHUT_RD / SHUT_WR / SHUT_RDWR

Source-side at `tcp.vr:1463-1467`. Tests pin the 3-variant
construction + 3 pairwise-disjointness checks.

### §3.3 `SOCKADDR_IN_SIZE` (16) and `SOCKADDR_IN6_SIZE` (28) are private

Source-side at `tcp.vr:1478-1481` as `const` (not `public`).
The values are hardcoded in the V-LLSI sockaddr serializer; on-
wire correctness is pinned indirectly via the IpAddr to_u32
round-trip tests in `core-tests/net/addr/`.

## 4. Action items landed in this branch

* `core-tests/net/tcp/unit_test.vr` — 6 unit tests covering
  Shutdown 3-variant construction (Read / Write / Both) + 3
  pairwise-disjointness checks.
* `core-tests/net/tcp/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| TcpStream.connect/.peer_addr/.local_addr against in-process loopback fixture | this folder + harness | 1 day |
| TcpListener.bind/.accept + accept_cancellable + reuseport | this folder | 1 day, gated on harness |
| set_nodelay / set_keepalive / set_linger socket-option round-trip via getsockopt | language level | 4h |
| AsyncRead/AsyncWrite poll_read/poll_write coverage | language level | 1 day |
| Expose SOCKADDR_IN_SIZE / SOCKADDR_IN6_SIZE as `public` for caller-side serialization | stdlib | 5 min |
