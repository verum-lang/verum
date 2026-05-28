# `core.sys.windows.winsock2` — implementation audit

## Status: **partial** (under `--interp`; constant + sockaddr surface, FFI deferred)

* Provides POSIX-compatible Winsock2 wrappers for cross-platform
  `core.net`.  Defines socket / connect / bind / listen / accept /
  accept4 / send / recv / sendto / recvfrom / shutdown / close / read /
  write / getsockname / getpeername / fcntl + 12 setsockopt helpers +
  multicast + peek / nonblock + WSADATA / WindowsSockaddrIn /
  WindowsSockaddrIn6 ABI shapes + 30+ POSIX-compat constants.
* WSA initialisation is lazy via WindowsOnce on first socket call.
* The FFI bindings (`@extern("ws2_32.dll")`) cannot run on a non-Windows
  host.

## 1. Cross-stdlib usage

| Caller | Use |
|---|---|
| `core.net.tcp` | Uses socket / connect / send / recv with `IPPROTO_TCP`. |
| `core.net.udp` | Uses socket / sendto / recvfrom with `IPPROTO_UDP`. |
| `core.net.unix` | NOT used — Unix sockets are Linux-only. |
| `core.net.cidr` | Uses the WindowsSockaddrIn / WindowsSockaddrIn6 binary layouts when constructing bind addresses. |

## 2. Pinned cross-platform invariants

The POSIX-compat names must agree with Linux on the values they share
AND must diverge correctly on the values that differ between platforms.

| Constant | Linux | Windows | Why |
|---|---|---|---|
| AF_INET | 2 | 2 | POSIX |
| AF_INET6 | 10 | **23** | Windows-specific |
| SOCK_STREAM | 1 | 1 | POSIX |
| SOCK_DGRAM | 2 | 2 | POSIX |
| SOCK_NONBLOCK | 0x800 | **0** | Windows handles non-blocking via ioctlsocket(FIONBIO), not as a socket() type flag |
| SOCK_CLOEXEC | 0x80000 | **0** | Windows handles inheritance via SetHandleInformation, not as a socket() type flag |
| IPPROTO_TCP | 6 | 6 | IANA |
| IPPROTO_UDP | 17 | 17 | IANA |
| SOL_SOCKET | 1 | **0xFFFF** | Windows-specific |
| SO_ERROR | 4 | **0x1007** | Windows-specific |
| SHUT_RD / SHUT_WR / SHUT_RDWR | 0 / 1 / 2 | 0 / 1 / 2 | POSIX |
| MSG_PEEK | 2 | 2 | POSIX |
| MSG_DONTWAIT | 0x40 | **0** | Windows lacks MSG_DONTWAIT; non-blocking via ioctlsocket |
| SOCKET_ERROR | -1 | -1 | POSIX-compat |
| INVALID_SOCKET | (n/a — Linux uses -1) | 0xFFFFFFFFFFFFFFFF | Windows SOCKET is unsigned UINT_PTR |

The bold-Windows values are the divergence catalogue — every value
that DIFFERS from Linux is pinned here.  Drift in either direction
would silently break cross-platform sockets at runtime.

## 3. Action items landed in this branch

1. `unit_test.vr` — 22 `@test`s pinning every constant from the table
   above plus shutdown-mode pairwise distinctness and
   `WindowsSockaddrIn` / `WindowsSockaddrIn6` record-shape round-trip
   (including the 16-byte addr buffer for IPv6 ::1).

## 4. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | socket / connect / send / recv / close round-trip via WSA | Requires Windows host with ws2_32.dll. |
| 2 | setsockopt_* helpers (SO_REUSEADDR / SO_KEEPALIVE / TCP_NODELAY / SO_RCVBUF / SO_SNDBUF / SO_RCVTIMEO / SO_SNDTIMEO) | Same gating. |
| 3 | Multicast (join_multicast_v4 / leave_multicast_v4 / set_multicast_ttl_v4 / set_multicast_loop_v4) | Same gating. |
| 4 | Non-blocking peek / recv_nonblock / send_nonblock | Same gating. |
| 5 | WSADATA layout round-trip | Same gating. |
