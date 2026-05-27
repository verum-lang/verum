# `net/udp` audit

Module: `core/net/udp.vr` (~1060 LOC) — UDP datagram sockets
via V-LLSI syscalls. The transport beneath DNS (RFC 1035 §4.2.1),
QUIC (transport for HTTP/3), multicast.

The module's public surface is intentionally focused: a single
`UdpSocket` record with bind / connect / send / recv / send_to /
recv_from + multicast join/leave + socket-option setters. There
is no public enum or data-surface algebra to cover from a USER
test module — every method needs a runtime fd.

Live tests live at `vcs/specs/L2-standard/net/udp/` against a
loopback harness.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.dns` resolver | UDP-first DNS queries with TCP fallback. |
| `core.net.quic` | base transport. |
| `core.net.http3` | UDP via QUIC. |
| Multicast network discovery | join_multicast_v4 / join_multicast_v6. |

## 2. Crate-side hardcodes

`crates/verum_runtime/src/net/udp/...` uses V-LLSI direct
syscalls (no libc).

## 3. Language-implementation gaps

### §3.1 UDP-1 — Functional surface gated on socket fixture

The UdpSocket public methods all require an OS-allocated UDP
socket fd. The fan-out methods (multicast, set_broadcast,
set_send_buffer_size) require either a per-platform socket-
option fixture or an emulation layer.

Pre-fix harness: none in `core-tests/`. End-to-end coverage at
language-level `vcs/specs/L2-standard/net/udp/`.

### §3.2 No public ADT to cover at data-surface

Unlike `core.net.tcp.Shutdown`, `core.net.unix.ShutdownKind`,
or `core.net.dns.DnsError`, `core.net.udp` exposes only
`UdpSocket` (a record with internal fields) — no public sum
type or constant to pin. The smoke test confirms the import
chain doesn't SIGSEGV under user-side compilation.

## 4. Action items landed in this branch

* `core-tests/net/udp/unit_test.vr` — 1 smoke test confirming
  the UdpSocket type symbol resolves through the import path
  without precompile-cascade SIGSEGV. (Equivalent of the
  CIDR-1 import-only probe.)
* `core-tests/net/udp/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| UdpSocket.bind/.send_to/.recv_from against in-process loopback | this folder + harness | 1 day |
| Multicast group join/leave coverage (IPv4 + IPv6) | this folder + multicast harness | 1 day |
| SO_REUSEPORT / SO_RCVBUF / SO_SNDBUF round-trip via getsockopt | language level | 4h |
| Add `enum UdpFlags` or similar public ADT to give a data-surface | stdlib design | 30 min discussion |
