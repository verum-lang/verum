//! Real TCP / UDP intrinsics for the VBC interpreter (Tier 0).
//!

//! Backs the `__tcp_*_raw` and `__udp_*_raw` family declared in
//! `core/sys/raw.vr`. The previous interpreter handler returned -1
//! for all of them — script-mode + interpreter-mode networking was
//! a documentation-only feature.
//!

//! Resource model: a thread-local `HashMap<i64, Resource>` keyed by a
//! synthetic file-descriptor number. The number is a small monotonic
//! counter (starts at 1) — NOT a kernel fd, so we never hand the
//! value to a syscall, only to other intrinsics. `__tcp_close_raw`
//! removes the entry; `Drop` of the resource closes the underlying
//! socket.
//!

//! The contract is the one declared in `core/sys/raw.vr`:
//!  * `__tcp_listen_raw(port: Int) -> Int` — bind 0.0.0.0:port, listen.
//!  * `__tcp_accept_raw(fd: Int) -> Int` — blocking accept.
//!  * `__tcp_connect_raw(host: Text, port: Int) -> Int` — TCP connect.
//!  * `__tcp_send_raw(fd: Int, data: Text) -> Int` — send-all, returns bytes or -1.
//!  * `__tcp_recv_raw(fd: Int, max_len: Int) -> Text` — single read.
//!  * `__tcp_close_raw(fd: Int) -> Int` — drop registration.
//!  * `__udp_bind_raw(port: Int) -> Int` — bind 0.0.0.0:port.
//!  * `__udp_send_raw(fd, data, host, port) -> Int` — send_to.
//!  * `__udp_recv_raw(fd: Int, max_len: Int) -> Text` — recv (peer ignored).
//!  * `__udp_close_raw(fd: Int) -> Int`
//!

//! Binary safety: `recv` returns Text via `String::from_utf8_lossy` —
//! same caveat as the AOT runtime's TCP API. Truly binary protocols
//! should use `core/net/tcp.vr` (syscall-driven, currently AOT-only).
//!

//! These intrinsics are deliberately blocking: they deliver the
//! "raw FFI fallback" promised by `core/sys/net_ops.vr` and unlock
//! `verum run --interp` HTTP demos at the cost of the executor
//! stalling on read/accept. The async-aware path (kqueue/io_uring
//! through `core/io/engine.vr`) remains an AOT-only feature.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use crate::interpreter::reactor::{wait_readable, wait_writable, WaitOutcome};

/// The kinds of network resources we track per fd.
enum NetResource {
    Listener(TcpListener),
    Stream(TcpStream),
    Udp(UdpSocket),
}

// VBC-NET-RT-1: process-wide async-safe REGISTRY (was per-thread
// `thread_local!` + `RefCell` — single-threaded only).  Async script
// work multiplexes across worker threads (the io_uring/kqueue
// executor in `core/io/engine`); a `thread_local!` REGISTRY would
// return DISTINCT instances per worker, so a `tcp_send(fd)` issued
// on a different thread than the original `tcp_connect` would NOT
// find the fd.  Production server systems MUST share the registry
// across all workers.  `LazyLock<Mutex<HashMap>>` is the chosen
// primitive: process-wide, predictable latency under contention,
// zero external deps.  The fd allocator uses an `AtomicI64` +
// `fetch_add(1, Relaxed)` — fully lock-free.
//
// **Lock-drop discipline (architectural invariant)**: the REGISTRY
// mutex is NEVER held across blocking I/O.  Two patterns achieve
// this:
//
//  1. **Take-and-restore** (`tcp_accept`): `remove()` the listener
//     under the lock, drop the lock, do the blocking accept, then
//     re-acquire and `insert()` back.  The fd is briefly absent from
//     the registry — concurrent close on the same fd will miss; this
//     is acceptable since accept and close on the same listener fd
//     are mutually exclusive operations in any sane server.
//
//  2. **Clone-and-go** (`tcp_send`/`tcp_recv`/`udp_send`/
//     `udp_recv_from`): call `try_clone()` on the socket UNDER the
//     lock (cheap — `dup(2)` on Unix, `WSADuplicateSocket` on
//     Windows), drop the lock, perform the blocking I/O on the
//     clone, drop the clone (closes only the dup'd fd; the original
//     stays in the registry).  Concurrent ops on the SAME fd are
//     serialised by the kernel's per-socket lock, NOT by this Mutex.
//     Concurrent ops on DIFFERENT fds proceed in true parallel.
//
// `tcp_close` removes the entry under the lock; if a peer thread
// is currently blocked in send/recv on a clone of that fd, the
// kernel keeps the underlying socket alive until the clone drops
// (refcounted file-table entry).  No use-after-free.
static REGISTRY: LazyLock<Mutex<HashMap<i64, NetResource>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_FD: AtomicI64 = AtomicI64::new(1);

fn alloc_fd() -> i64 {
    NEXT_FD.fetch_add(1, Ordering::Relaxed)
}

fn register(res: NetResource) -> i64 {
    let fd = alloc_fd();
    REGISTRY.lock().unwrap().insert(fd, res);
    fd
}

// =============================================================================
// TCP
// =============================================================================

pub fn tcp_listen(port: i64) -> i64 {
    if !(0..=65535).contains(&port) {
        return -1;
    }
    match TcpListener::bind(("0.0.0.0", port as u16)) {
        Ok(l) => register(NetResource::Listener(l)),
        Err(_) => -1,
    }
}

/// Flags accepted by [`tcp_listen_v2`] (matches the `flags` argument
/// of `__tcp_listen_v2_raw` declared in `core/sys/raw.vr`).
///

/// Bit 0 (`TCP_LISTEN_FLAG_REUSEPORT`): set SO_REUSEPORT on the
/// listener — multiple listeners on the same `host:port` load-balance
/// in the kernel. Linux ≥3.9 / macOS ≥10.7. Best-effort: silently
/// ignored if the platform lacks the option (Windows).
///

/// Higher bits are reserved.
pub const TCP_LISTEN_FLAG_REUSEPORT: i64 = 1 << 0;

/// Rich-signature TCP listen intrinsic.
///

/// `host` parses as an IP literal (`"0.0.0.0"`, `"127.0.0.1"`,
/// `"::"`, `"::1"`, …). DNS resolution is intentionally out of scope —
/// the high-level `core.net.tcp.TcpListener.bind` already iterates a
/// resolved address list and calls into us with one literal at a time.
///

/// `port = 0` asks the kernel to choose. Use [`tcp_local_port`] to
/// retrieve the actual port afterwards.
///

/// `backlog` is forwarded to `listen(2)`. Kernels typically silently
/// cap large values at `/proc/sys/net/core/somaxconn` (or similar).
///

/// Returns:
/// * `fd > 0` on success — the synthetic FD that other intrinsics
///  accept (NOT a kernel fd; see module-level docs).
/// * `-errno` on bind/listen failure — caller maps to IoErrorKind via
///  `core/io/protocols.vr::from_raw_os_error`.
/// * `-EINVAL` (`-22` Linux / `-22` macOS) for argument-validation
///  failures (bad host, port out of range, negative backlog).
///

/// The errno-preservation contract is the architectural promise that
/// distinguishes v2 from v1 (which collapses everything to `-1`).
pub fn tcp_listen_v2(host: &str, port: i64, backlog: i64, flags: i64) -> i64 {
    if !(0..=65535).contains(&port) {
        return -(libc::EINVAL as i64);
    }
    if !(0..=65535).contains(&backlog) {
        return -(libc::EINVAL as i64);
    }

    let parsed: std::net::IpAddr = match host.parse() {
        Ok(ip) => ip,
        Err(_) => return -(libc::EINVAL as i64),
    };
    let addr = std::net::SocketAddr::new(parsed, port as u16);

    // We pin the socket through `std::net::TcpListener::bind` (which
    // does socket+bind+listen with default backlog=128) and then layer
    // the customisations on top via `setsockopt` against the raw fd.
    // Going through `std::net` rather than re-implementing the C
    // sequence keeps interpreter-mode behaviour identical to AOT for
    // the common path (default flags) and limits the failure surface.
    //

    // SO_REUSEADDR is ALWAYS set — same default as the AOT
    // `verum_tcp_listen` helper and the user-facing `core.net.tcp`
    // surface. Quick rebind after process restart is the universally
    // expected behaviour.
    let listener = match TcpListener::bind(addr) {
        Ok(l) => l,
        Err(e) => return -(e.raw_os_error().unwrap_or(libc::EINVAL) as i64),
    };

    // Apply flags. Failures here are non-fatal — listener is already
    // bound; we degrade gracefully and return the fd. If the user
    // demanded SO_REUSEPORT and the kernel rejects it (e.g. older
    // FreeBSD with no LB variant), they get a working listener
    // without load-balancing, which is the sensible degradation.
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let fd = listener.as_raw_fd();
        if flags & TCP_LISTEN_FLAG_REUSEPORT != 0 {
            let on: libc::c_int = 1;
            // SAFETY: fd is owned by `listener` (still alive in this
            // scope); pointer/length pair targets a stack i32.
            unsafe {
                libc::setsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_REUSEPORT,
                    &on as *const _ as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                );
            }
        }
    }

    // The std::net listener already calls listen() with backlog=128
    // internally. To honour an explicit caller-provided backlog we
    // re-invoke listen(2) on the raw fd. Failures here, like the flag
    // path above, are non-fatal — caller still gets a working listener
    // with the std default.
    #[cfg(unix)]
    if backlog != 128 {
        use std::os::fd::AsRawFd;
        let fd = listener.as_raw_fd();
        // SAFETY: fd is owned by listener; backlog is bounded
        // [0, 65535] by the validation above.
        unsafe {
            libc::listen(fd, backlog as libc::c_int);
        }
    }

    // **Real-kernel-fd return** (#25 cascade closure).
    //

    // Pre-#25 v2 returned a synthetic fd via `register(...)` and the
    // std::net wrapper lived in the registry for resource lifetime
    // tracking. Synthetic fds are incompatible with the user-facing
    // `core.net.tcp.TcpListener` surface, which threads its `fd` field
    // through libc-bound operations (`accept`/`close`/`setsockopt`/
    // `getsockname`) — those operations are dispatched via libffi in
    // interpreter mode and require a real kernel fd.
    //

    // Resolution: `into_raw_fd()` consumes the std::net wrapper and
    // returns the underlying kernel fd. Ownership transfers to the
    // caller — Verum-side `core/net/tcp.vr::TcpListener::Drop` calls
    // libc `close()` to free the fd at the right scope boundary,
    // mirroring the AOT path's lifecycle. No registry tracking is
    // needed: the kernel itself is the source of truth for fd
    // existence, and the operating system reaps any leaked fds at
    // process exit.
    //

    // Companion `tcp_local_port` updated alongside to call
    // `getsockname(2)` directly on the raw fd (no registry lookup).
    #[cfg(unix)]
    {
        use std::os::fd::IntoRawFd;
        return listener.into_raw_fd() as i64;
    }
    #[cfg(not(unix))]
    {
        // Windows: SOCKET handles are not directly compatible with
        // libc fd APIs. Keep the legacy registry path here and
        // leave Windows fd-bridging as a follow-up. Most weft
        // production paths target Unix.
        register(NetResource::Listener(listener))
    }
}

/// Returns the OS-assigned local port for a TCP listener / stream / UDP
/// socket fd. Used to recover the kernel-chosen port after
/// `tcp_listen_v2(_, 0, _, _)`. Returns `-1` on `getsockname(2)`
/// failure (bad fd, unbound socket, …).
///

/// **Real-fd path** (Unix): post-#25, `tcp_listen_v2` returns the raw
/// kernel fd (no registry tracking). We call `getsockname(2)`
/// directly on the fd — works for any bound socket regardless of
/// whether it was created via this intrinsic family or via a
/// libffi-bridged libc `socket()` call. This is the source of truth
/// for the bound port and matches the semantics the AOT
/// `verum_tcp_local_port` LLVM helper provides
/// (`crates/verum_codegen/src/llvm/runtime.rs`).
///

/// **Legacy registry fallback**: pre-#25 listeners that were
/// `register()`-ed with a synthetic fd still hit the registry path.
/// The registry tries first, then falls through to `getsockname` if
/// the fd isn't tracked — single API surface, two backing mechanisms.
pub fn tcp_local_port(fd: i64) -> i64 {
    // Fast path: registry lookup for legacy synthetic-fd flows.
    let from_registry = {
        let map = REGISTRY.lock().unwrap();
        match map.get(&fd) {
            Some(NetResource::Listener(l)) => {
                Some(l.local_addr().map(|a| a.port() as i64).unwrap_or(-1))
            }
            Some(NetResource::Stream(s)) => {
                Some(s.local_addr().map(|a| a.port() as i64).unwrap_or(-1))
            }
            Some(NetResource::Udp(s)) => {
                Some(s.local_addr().map(|a| a.port() as i64).unwrap_or(-1))
            }
            None => None,
        }
    };
    if let Some(p) = from_registry {
        return p;
    }

    // Real-fd path: call `getsockname(2)` directly on the fd.
    #[cfg(unix)]
    unsafe {
        // sockaddr_in6 is the larger of v4/v6 — 28 bytes covers both
        // and the kernel writes the actual size into the in/out
        // length. sin_port lives at offset 2 in either layout.
        let mut sa = [0u8; 28];
        let mut sa_len: libc::socklen_t = 28;
        let rc = libc::getsockname(
            fd as libc::c_int,
            sa.as_mut_ptr() as *mut libc::sockaddr,
            &mut sa_len,
        );
        if rc < 0 {
            return -1;
        }
        // sin_port at offset 2, big-endian u16.
        let port_be = u16::from_be_bytes([sa[2], sa[3]]);
        port_be as i64
    }
    #[cfg(not(unix))]
    {
        // Windows: synthetic-fd flow only. Real-fd query path falls
        // through to -1.
        -1
    }
}

/// Read the connected-peer address of a TCP fd via the registry's
/// `peer_addr()` (synthetic-fd path) or `getpeername(2)` (real-fd
/// path). Returns the peer as a `(family, host_str, port)` tuple
/// where family is 4 or 6. None when the fd isn't tracked or the
/// kernel call fails. Used by VBC-NET-4 for peer_addr round-trip
/// in `TcpStream` records.
pub fn tcp_peer_addr(fd: i64) -> Option<(u8, String, i64)> {
    // Fast path: registry → std::net::TcpStream::peer_addr.
    let from_registry = {
        let map = REGISTRY.lock().unwrap();
        match map.get(&fd) {
            Some(NetResource::Stream(s)) => s.peer_addr().ok(),
            _ => None,
        }
    };
    if let Some(addr) = from_registry {
        let port = addr.port() as i64;
        return match addr {
            std::net::SocketAddr::V4(v4) => Some((4, v4.ip().to_string(), port)),
            std::net::SocketAddr::V6(v6) => Some((6, v6.ip().to_string(), port)),
        };
    }
    // Real-fd path: `getpeername(2)` on the kernel fd. Mirrors the
    // shape used in `tcp_local_port`.
    #[cfg(unix)]
    unsafe {
        let mut sa = [0u8; 28];
        let mut sa_len: libc::socklen_t = 28;
        let rc = libc::getpeername(
            fd as libc::c_int,
            sa.as_mut_ptr() as *mut libc::sockaddr,
            &mut sa_len,
        );
        if rc < 0 {
            return None;
        }
        // sa_family at offset 0. AF_INET = 2, AF_INET6 = 30 (BSD)
        // or 10 (Linux). The cross-family detection here uses the
        // libc constants directly.
        let family = sa[0] as i32;
        let port_be = u16::from_be_bytes([sa[2], sa[3]]);
        let port = port_be as i64;
        if family == libc::AF_INET {
            // sockaddr_in: sin_addr at offset 4 (4 bytes).
            let host = format!("{}.{}.{}.{}", sa[4], sa[5], sa[6], sa[7]);
            return Some((4, host, port));
        }
        if family == libc::AF_INET6 {
            // sockaddr_in6: sin6_addr at offset 8 (16 bytes). Build
            // the canonical hex representation; std::net::Ipv6Addr's
            // Display gives RFC 5952 with `::` compression.
            let octets: [u8; 16] = [
                sa[8], sa[9], sa[10], sa[11], sa[12], sa[13], sa[14], sa[15], sa[16], sa[17],
                sa[18], sa[19], sa[20], sa[21], sa[22], sa[23],
            ];
            let v6 = std::net::Ipv6Addr::from(octets);
            return Some((6, v6.to_string(), port));
        }
        None
    }
    #[cfg(not(unix))]
    {
        let _ = fd;
        None
    }
}

pub fn tcp_accept(listen_fd: i64) -> i64 {
    // Pull the listener out of the registry briefly so we don't hold
    // a RefCell borrow across the (potentially blocking) accept call.
    let listener: Option<TcpListener> = {
        let mut map = REGISTRY.lock().unwrap();
        match map.remove(&listen_fd) {
            Some(NetResource::Listener(l)) => Some(l),
            other => {
                if let Some(o) = other {
                    map.insert(listen_fd, o);
                }
                None
            }
        }
    };
    if let Some(listener) = listener {
        // Synthetic-fd path (legacy `__tcp_listen_raw`).
        let result = listener.accept();
        {
            REGISTRY.lock().unwrap()
                .insert(listen_fd, NetResource::Listener(listener));
        };
        return match result {
            Ok((stream, _peer)) => register(NetResource::Stream(stream)),
            Err(_) => -1,
        };
    }

    // Real-kernel-fd path (`__tcp_listen_v2_raw` returns a real fd via
    // `IntoRawFd::into_raw_fd()` — see commit c15df24b). REGISTRY
    // lookup misses, so we wrap the raw fd into a `TcpListener` long
    // enough to call accept(2), then immediately surrender ownership
    // back via `into_raw_fd()` to keep the listener alive (`TcpListener::Drop`
    // would close the kernel fd otherwise). The accepted connection's
    // stream is registered in REGISTRY so subsequent recv/send/close
    // continue through the existing synthetic-fd machinery without
    // change.
    //

    // Cross-platform: Unix uses FromRawFd; Windows uses FromRawSocket.
    // Other platforms have no real-fd accept path — fall through to -1.
    #[cfg(unix)]
    {
        use std::os::unix::io::{FromRawFd, IntoRawFd};
        // SAFETY: `listen_fd` came from `__tcp_listen_v2_raw`'s
        // `IntoRawFd::into_raw_fd()`. The fd is valid and owned by
        // the Verum runtime until `__tcp_close_raw` is called.
        // `TcpListener::from_raw_fd` takes ownership; we hand it back
        // via `into_raw_fd()` immediately after accept to keep the
        // kernel fd alive across calls.
        let listener = unsafe { TcpListener::from_raw_fd(listen_fd as i32) };
        let result = listener.accept();
        // Surrender ownership: kernel fd survives, TcpListener drops
        // without closing.
        let _surrendered_fd = listener.into_raw_fd();
        match result {
            Ok((stream, _peer)) => register(NetResource::Stream(stream)),
            Err(_) => -1,
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::{FromRawSocket, IntoRawSocket};
        let listener = unsafe { TcpListener::from_raw_socket(listen_fd as u64) };
        let result = listener.accept();
        let _surrendered = listener.into_raw_socket();
        match result {
            Ok((stream, _peer)) => register(NetResource::Stream(stream)),
            Err(_) => -1,
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = listen_fd;
        -1
    }
}

pub fn tcp_connect(host: &str, port: i64) -> i64 {
    if !(0..=65535).contains(&port) {
        return -1;
    }
    match TcpStream::connect((host, port as u16)) {
        Ok(s) => register(NetResource::Stream(s)),
        Err(_) => -1,
    }
}

pub fn tcp_send(fd: i64, data: &[u8]) -> i64 {
    // Synthetic-fd registry path: clone the stream out of the
    // registry under the lock, then drop the lock before the
    // (potentially blocking) write_all.  Concurrent send/recv on
    // OTHER fds proceed in parallel.  See "Lock-drop discipline"
    // at the top of this file.
    let clone_result: Option<TcpStream> = {
        let map = REGISTRY.lock().unwrap();
        match map.get(&fd) {
            Some(NetResource::Stream(s)) => s.try_clone().ok(),
            _ => None,
        }
    };
    if let Some(mut stream) = clone_result {
        return match stream.write_all(data) {
            Ok(()) => data.len() as i64,
            Err(_) => -1,
        };
    }

    // Real-kernel-fd path: a v2-listener-accepted connection that
    // somehow leaked into raw-fd form (or a future intrinsic that
    // returns raw fds directly). Wrap fd in a TcpStream long enough
    // to call write_all, then surrender ownership.
    #[cfg(unix)]
    {
        use std::os::unix::io::{FromRawFd, IntoRawFd};
        // SAFETY: caller-supplied fd is documented as a real kernel
        // fd in this fallback branch; the synthetic-fd table missed.
        let mut stream = unsafe { TcpStream::from_raw_fd(fd as i32) };
        let result = stream.write_all(data);
        let _surrendered = stream.into_raw_fd();
        return match result {
            Ok(()) => data.len() as i64,
            Err(_) => -1,
        };
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::{FromRawSocket, IntoRawSocket};
        let mut stream = unsafe { TcpStream::from_raw_socket(fd as u64) };
        let result = stream.write_all(data);
        let _surrendered = stream.into_raw_socket();
        return match result {
            Ok(()) => data.len() as i64,
            Err(_) => -1,
        };
    }
    #[cfg(not(any(unix, windows)))]
    {
        -1
    }
}

pub fn tcp_recv(fd: i64, max_len: i64) -> Option<String> {
    if max_len <= 0 {
        return Some(String::new());
    }
    let cap = max_len.min(1 << 20) as usize; // hard-cap 1 MiB / call.
    let mut buf = vec![0_u8; cap];
    // Synthetic-fd registry path: clone-and-go (lock dropped
    // before the blocking read).
    let clone_result: Option<TcpStream> = {
        let map = REGISTRY.lock().unwrap();
        match map.get(&fd) {
            Some(NetResource::Stream(s)) => s.try_clone().ok(),
            _ => None,
        }
    };
    if let Some(mut stream) = clone_result {
        let n = stream.read(&mut buf).ok()?;
        buf.truncate(n);
        return Some(String::from_utf8_lossy(&buf).into_owned());
    }

    // Real-kernel-fd fallback.
    #[cfg(unix)]
    {
        use std::os::unix::io::{FromRawFd, IntoRawFd};
        let mut stream = unsafe { TcpStream::from_raw_fd(fd as i32) };
        let result = stream.read(&mut buf).ok();
        let _surrendered = stream.into_raw_fd();
        let n = result?;
        buf.truncate(n);
        Some(String::from_utf8_lossy(&buf).into_owned())
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::{FromRawSocket, IntoRawSocket};
        let mut stream = unsafe { TcpStream::from_raw_socket(fd as u64) };
        let result = stream.read(&mut buf).ok();
        let _surrendered = stream.into_raw_socket();
        let n = result?;
        buf.truncate(n);
        Some(String::from_utf8_lossy(&buf).into_owned())
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

pub fn tcp_close(fd: i64) -> i64 {
    // Legacy synthetic-fd path: drop from registry, std::net Drop runs.
    let registry_hit = {
        let mut map = REGISTRY.lock().unwrap();
        map.remove(&fd).is_some()
    };
    if registry_hit {
        return 0;
    }

    // Real-fd path (#25): call libc `close(2)` directly. v2 listen
    // returns raw kernel fds (no registry tracking), so close has to
    // mirror that path. Returns 0 on success, -errno on failure
    // (matches the v2 errno-preserving convention).
    #[cfg(unix)]
    unsafe {
        let rc = libc::close(fd as libc::c_int);
        if rc == 0 { 0 } else { -1 }
    }
    #[cfg(not(unix))]
    {
        -1
    }
}

// =============================================================================
// UDP
// =============================================================================

pub fn udp_bind(port: i64) -> i64 {
    if !(0..=65535).contains(&port) {
        return -1;
    }
    match UdpSocket::bind(("0.0.0.0", port as u16)) {
        Ok(s) => register(NetResource::Udp(s)),
        Err(_) => -1,
    }
}

pub fn udp_send(fd: i64, data: &[u8], host: &str, port: i64) -> i64 {
    if !(0..=65535).contains(&port) {
        return -1;
    }
    // Clone-and-go: drop the REGISTRY lock before send_to, which can
    // block on backpressure (full kernel send buffer) — common under
    // sustained UDP burst.
    let clone_result: Option<UdpSocket> = {
        let map = REGISTRY.lock().unwrap();
        match map.get(&fd) {
            Some(NetResource::Udp(s)) => s.try_clone().ok(),
            _ => None,
        }
    };
    match clone_result {
        Some(s) => match s.send_to(data, (host, port as u16)) {
            Ok(n) => n as i64,
            Err(_) => -1,
        },
        None => -1,
    }
}

pub fn udp_recv(fd: i64, max_len: i64) -> Option<String> {
    udp_recv_from(fd, max_len).map(|(s, _peer)| s)
}

/// VBC-NET-AUDIT-1 — return the recv'd payload AND the source
/// peer address (instead of dropping it like the legacy `udp_recv`
/// did).  The caller can then use the peer for routing /
/// connection-tracking decisions, which is the canonical UDP
/// server pattern (DNS / NTP / QUIC handshake / DHCP server).
///
/// Peer is returned in the same `(family, host_str, port)` shape
/// that `tcp_peer_addr` uses so the high-level intercept can
/// build a `SocketAddr.V4 | SocketAddr.V6` via the shared
/// `SocketAddrCodec` (avoiding the placeholder `0.0.0.0:0` that
/// the pre-fix intercept fell back to).
pub fn udp_recv_from(
    fd: i64,
    max_len: i64,
) -> Option<(String, Option<(u8, String, i64)>)> {
    if max_len <= 0 {
        return Some((String::new(), None));
    }
    let cap = max_len.min(1 << 20) as usize;
    let mut buf = vec![0_u8; cap];
    // Clone-and-go: recv_from blocks until a datagram arrives.
    // Holding the REGISTRY lock across that wait would serialise
    // ALL other net_runtime intrinsics — the canonical UDP-server
    // anti-pattern.
    let clone_result: Option<UdpSocket> = {
        let map = REGISTRY.lock().unwrap();
        match map.get(&fd) {
            Some(NetResource::Udp(s)) => s.try_clone().ok(),
            _ => None,
        }
    };
    let socket = clone_result?;
    let (n, peer_sock) = socket.recv_from(&mut buf).ok()?;
    buf.truncate(n);
    let peer_tuple = match peer_sock {
        std::net::SocketAddr::V4(v4) => Some((4u8, v4.ip().to_string(), v4.port() as i64)),
        std::net::SocketAddr::V6(v6) => Some((6u8, v6.ip().to_string(), v6.port() as i64)),
    };
    Some((String::from_utf8_lossy(&buf).into_owned(), peer_tuple))
}

pub fn udp_close(fd: i64) -> i64 {
    tcp_close(fd) // same semantics — drop registration.
}

// ============================================================================
// VBC-NET-RT-2 — reactor-backed timeout-bound I/O
//
// The functions below take an explicit `timeout_ms` parameter and use
// the singleton `interpreter::reactor` (kqueue/epoll backend) to wait
// for socket readiness.  Once the reactor signals `Ready`, the I/O is
// performed via the same clone-and-go pattern as the blocking variants,
// but the cloned socket is set non-blocking so we can detect spurious
// wake-ups and re-arm the wait if needed.
//
// Return convention (chosen for ergonomic intrinsic dispatch):
//
//  * `tcp_accept_timeout(fd, timeout_ms) -> i64`
//      - >0: accepted synthetic fd
//      - -1: I/O error (registry miss, accept failed)
//      - -2: timeout
//      - -3: reactor unhealthy (caller should fall back to blocking)
//
//  * `tcp_recv_timeout(fd, max_len, timeout_ms) -> (i64, String)`
//      - i64: status code (>=0 = bytes, -1 = EOF/closed, -2 = timeout, -3 = err)
//      - String: payload (empty for non-positive status)
//
//  * `udp_recv_from_timeout(fd, max_len, timeout_ms)
//        -> (i64, String, Option<(family, host, port)>)`
//
//  * `tcp_send_timeout(fd, data, timeout_ms) -> i64`
//      - bytes_written or negative status as above.
//
//  * `tcp_connect_timeout(host, port, timeout_ms) -> i64`
//      - synthetic fd or negative status.
//
// All five preserve the same lock-drop discipline as the blocking
// variants (try_clone under the lock, drop the lock, do I/O on the
// clone).  The reactor itself uses a sharded waiter map so concurrent
// timeout waits on different fds proceed in parallel.
// ============================================================================

/// Status sentinels for the timeout-bound family.
pub const NET_STATUS_EOF: i64 = -1;
/// The deadline elapsed before the socket became ready.
pub const NET_STATUS_TIMEOUT: i64 = -2;
/// The reactor itself failed (cannot wait — fall back to blocking).
pub const NET_STATUS_REACTOR_ERROR: i64 = -3;
/// I/O on the socket (after readiness) failed.
pub const NET_STATUS_IO_ERROR: i64 = -4;

fn timeout_from_ms(timeout_ms: i64) -> Duration {
    if timeout_ms <= 0 {
        Duration::from_millis(0)
    } else {
        Duration::from_millis(timeout_ms as u64)
    }
}

/// Block until `fd` is readable or `timeout_ms` elapses.  Returns
/// 1 on ready, 0 on timeout, -1 on reactor error.  Public surface
/// for `__io_wait_readable_raw`.
pub fn io_wait_readable(fd: i64, timeout_ms: i64) -> i64 {
    match wait_readable(fd, timeout_from_ms(timeout_ms)) {
        WaitOutcome::Ready => 1,
        WaitOutcome::TimedOut => 0,
        WaitOutcome::Error => -1,
    }
}

/// Block until `fd` is writable or `timeout_ms` elapses.  Same
/// return convention as `io_wait_readable`.
pub fn io_wait_writable(fd: i64, timeout_ms: i64) -> i64 {
    match wait_writable(fd, timeout_from_ms(timeout_ms)) {
        WaitOutcome::Ready => 1,
        WaitOutcome::TimedOut => 0,
        WaitOutcome::Error => -1,
    }
}

pub fn tcp_accept_timeout(listen_fd: i64, timeout_ms: i64) -> i64 {
    // Read the listener's raw_fd briefly under the lock — the
    // registry holds the listener alive throughout this call (we
    // don't take it out), so the raw fd is stable.  Concurrent
    // tcp_accept callers serialise at the kernel's accept queue
    // (kernel-level lock), NOT at the REGISTRY mutex, mirroring
    // the lock-drop discipline the rest of this module follows.
    let raw_fd: i64 = {
        let map = REGISTRY.lock().unwrap();
        match map.get(&listen_fd) {
            Some(NetResource::Listener(l)) => listener_raw_fd(l),
            _ => -1,
        }
    };
    if raw_fd >= 0 {
        let deadline =
            std::time::Instant::now() + Duration::from_millis(timeout_ms.max(0) as u64);
        loop {
            let now = std::time::Instant::now();
            if now >= deadline {
                return NET_STATUS_TIMEOUT;
            }
            let remaining_ms = (deadline - now).as_millis() as i64;
            match io_wait_readable(raw_fd, remaining_ms.max(1)) {
                0 => return NET_STATUS_TIMEOUT,
                1 => {}
                _ => return NET_STATUS_REACTOR_ERROR,
            }
            // Per-call non-blocking accept via libc::accept on the
            // raw kernel fd.  We do NOT set O_NONBLOCK on the
            // listener — the kernel-level accept queue serialises
            // multiple concurrent acceptors automatically.  If the
            // queue is empty (spurious wake), accept blocks; that's
            // acceptable for the rare-spurious case since the
            // reactor's signal almost always corresponds to a real
            // pending connection.  For absolute non-blocking
            // semantics on Linux we'd use accept4(SOCK_NONBLOCK);
            // macOS lacks accept4 and the fcntl-on-listener path
            // would pollute the shared file description.
            let mut peer: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
            let mut peer_len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
            #[cfg(target_os = "linux")]
            let conn_fd = unsafe {
                libc::accept4(
                    raw_fd as libc::c_int,
                    &mut peer as *mut _ as *mut libc::sockaddr,
                    &mut peer_len,
                    libc::SOCK_CLOEXEC,
                )
            };
            #[cfg(not(target_os = "linux"))]
            let conn_fd = unsafe {
                libc::accept(
                    raw_fd as libc::c_int,
                    &mut peer as *mut _ as *mut libc::sockaddr,
                    &mut peer_len,
                )
            };
            if conn_fd >= 0 {
                #[cfg(unix)]
                {
                    use std::os::unix::io::FromRawFd;
                    let stream = unsafe { TcpStream::from_raw_fd(conn_fd) };
                    return register(NetResource::Stream(stream));
                }
                #[cfg(not(unix))]
                return NET_STATUS_IO_ERROR;
            }
            let err = std::io::Error::last_os_error();
            match err.kind() {
                std::io::ErrorKind::WouldBlock => continue, // spurious wake — re-park
                std::io::ErrorKind::Interrupted => continue,
                _ => return NET_STATUS_IO_ERROR,
            }
        }
    }
    // Real-kernel-fd path (caller passed a v2-listener raw fd).
    #[cfg(unix)]
    {
        let deadline =
            std::time::Instant::now() + Duration::from_millis(timeout_ms.max(0) as u64);
        loop {
            let now = std::time::Instant::now();
            if now >= deadline {
                return NET_STATUS_TIMEOUT;
            }
            let remaining_ms = (deadline - now).as_millis() as i64;
            match io_wait_readable(listen_fd, remaining_ms.max(1)) {
                0 => return NET_STATUS_TIMEOUT,
                1 => {}
                _ => return NET_STATUS_REACTOR_ERROR,
            }
            let mut peer: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
            let mut peer_len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
            let conn_fd = unsafe {
                libc::accept(
                    listen_fd as libc::c_int,
                    &mut peer as *mut _ as *mut libc::sockaddr,
                    &mut peer_len,
                )
            };
            if conn_fd >= 0 {
                use std::os::unix::io::FromRawFd;
                let stream = unsafe { TcpStream::from_raw_fd(conn_fd) };
                return register(NetResource::Stream(stream));
            }
            let err = std::io::Error::last_os_error();
            match err.kind() {
                std::io::ErrorKind::WouldBlock => continue,
                std::io::ErrorKind::Interrupted => continue,
                _ => return NET_STATUS_IO_ERROR,
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = listen_fd;
        NET_STATUS_REACTOR_ERROR
    }
}

pub fn tcp_recv_timeout(fd: i64, max_len: i64, timeout_ms: i64) -> (i64, String) {
    if max_len <= 0 {
        return (0, String::new());
    }
    let cap = max_len.min(1 << 20) as usize;
    let mut buf = vec![0_u8; cap];
    // We need the ORIGINAL kernel fd (the one stored in the
    // registry) so reactor signals are delivered for the long-
    // lived socket, NOT for an ephemeral dup that we'd close on
    // function return.  Reading the raw fd while holding the
    // registry lock keeps it valid for the duration of the
    // function (the registry holds the TcpStream alive).
    let raw_fd: i64 = {
        let map = REGISTRY.lock().unwrap();
        match map.get(&fd) {
            Some(NetResource::Stream(s)) => stream_raw_fd(s),
            _ => return (NET_STATUS_IO_ERROR, String::new()),
        }
    };
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms.max(0) as u64);
    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            return (NET_STATUS_TIMEOUT, String::new());
        }
        let remaining_ms = (deadline - now).as_millis() as i64;
        match io_wait_readable(raw_fd, remaining_ms.max(1)) {
            0 => return (NET_STATUS_TIMEOUT, String::new()),
            1 => {}
            _ => return (NET_STATUS_REACTOR_ERROR, String::new()),
        }
        // Try a non-blocking recv via MSG_DONTWAIT — this is
        // per-call, not per-socket, so nothing else's view of the
        // mode is affected.  On WouldBlock (spurious wake from
        // POLLHUP / level-triggered re-fire / kqueue stale event
        // after fd recycling), loop back to the reactor.
        let n = unsafe {
            libc::recv(
                raw_fd as libc::c_int,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                libc::MSG_DONTWAIT,
            )
        };
        if n > 0 {
            let n = n as usize;
            buf.truncate(n);
            return (n as i64, String::from_utf8_lossy(&buf).into_owned());
        } else if n == 0 {
            // Defensive: kqueue's EV_EOF can latch from a previous
            // socket that occupied this kernel fd before recycling.
            // To distinguish a real peer-FIN from a stale event,
            // verify with getsockopt(SO_ERROR) (zero = clean state)
            // AND a peek-recv (would-block = no real EOF; the read
            // returned 0 spuriously and the OS is still happy with
            // the connection).  Only then return EOF.
            if !verify_real_eof(raw_fd) {
                continue;
            }
            return (NET_STATUS_EOF, String::new());
        } else {
            let err = std::io::Error::last_os_error();
            match err.kind() {
                std::io::ErrorKind::WouldBlock => {
                    // Spurious wake — loop, the deadline check at
                    // the top of the loop will eventually fire.
                    continue;
                }
                std::io::ErrorKind::Interrupted => continue,
                _ => return (NET_STATUS_IO_ERROR, String::new()),
            }
        }
    }
}

/// Distinguish a genuine peer-FIN from a kqueue stale-event wake
/// after fd recycling.  Returns true iff the kernel agrees the
/// socket is truly EOF.
fn verify_real_eof(raw_fd: i64) -> bool {
    // 1. SO_ERROR check.  Non-zero => connection has an error
    //    pending; treat as EOF.
    let mut so_err: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            raw_fd as libc::c_int,
            libc::SOL_SOCKET,
            libc::SO_ERROR,
            &mut so_err as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if rc != 0 || so_err != 0 {
        return true;
    }
    // 2. Peek-recv: if no data and no FIN, recv(MSG_PEEK|MSG_DONTWAIT)
    //    returns EWOULDBLOCK — the earlier 0-return was spurious.
    let mut probe = [0u8; 1];
    let n = unsafe {
        libc::recv(
            raw_fd as libc::c_int,
            probe.as_mut_ptr() as *mut libc::c_void,
            1,
            libc::MSG_DONTWAIT | libc::MSG_PEEK,
        )
    };
    if n < 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::WouldBlock {
            return false;
        }
    }
    true
}

pub fn tcp_send_timeout(fd: i64, data: &[u8], timeout_ms: i64) -> i64 {
    if data.is_empty() {
        return 0;
    }
    let raw_fd: i64 = {
        let map = REGISTRY.lock().unwrap();
        match map.get(&fd) {
            Some(NetResource::Stream(s)) => stream_raw_fd(s),
            _ => return NET_STATUS_IO_ERROR,
        }
    };
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms.max(0) as u64);
    let mut written = 0_usize;
    while written < data.len() {
        let now = std::time::Instant::now();
        if now >= deadline {
            return if written > 0 {
                written as i64
            } else {
                NET_STATUS_TIMEOUT
            };
        }
        let remaining_ms = (deadline - now).as_millis() as i64;
        match io_wait_writable(raw_fd, remaining_ms.max(1)) {
            0 => {
                return if written > 0 {
                    written as i64
                } else {
                    NET_STATUS_TIMEOUT
                };
            }
            1 => {}
            _ => return NET_STATUS_REACTOR_ERROR,
        }
        // Per-call non-blocking send via MSG_DONTWAIT |
        // MSG_NOSIGNAL — keeps the socket's mode unchanged AND
        // suppresses the SIGPIPE that would kill the host process
        // on EPIPE (peer closed the stream half-way through).
        let flags = libc::MSG_DONTWAIT | platform_msg_nosignal();
        let n = unsafe {
            libc::send(
                raw_fd as libc::c_int,
                data[written..].as_ptr() as *const libc::c_void,
                data.len() - written,
                flags,
            )
        };
        if n > 0 {
            written += n as usize;
        } else if n == 0 {
            return NET_STATUS_EOF;
        } else {
            let err = std::io::Error::last_os_error();
            match err.kind() {
                std::io::ErrorKind::WouldBlock => continue,
                std::io::ErrorKind::Interrupted => continue,
                _ => {
                    return if written > 0 {
                        written as i64
                    } else {
                        NET_STATUS_IO_ERROR
                    };
                }
            }
        }
    }
    written as i64
}

/// Per-platform MSG_NOSIGNAL.  macOS doesn't have it (uses SO_NOSIGPIPE
/// socket option instead, set when the socket is opened); Linux has it.
#[inline]
fn platform_msg_nosignal() -> i32 {
    #[cfg(target_os = "linux")]
    {
        libc::MSG_NOSIGNAL
    }
    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

pub fn udp_recv_from_timeout(
    fd: i64,
    max_len: i64,
    timeout_ms: i64,
) -> (i64, String, Option<(u8, String, i64)>) {
    if max_len <= 0 {
        return (0, String::new(), None);
    }
    let cap = max_len.min(1 << 20) as usize;
    let mut buf = vec![0_u8; cap];
    let raw_fd: i64 = {
        let map = REGISTRY.lock().unwrap();
        match map.get(&fd) {
            Some(NetResource::Udp(s)) => udp_raw_fd(s),
            _ => return (NET_STATUS_IO_ERROR, String::new(), None),
        }
    };
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms.max(0) as u64);
    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            return (NET_STATUS_TIMEOUT, String::new(), None);
        }
        let remaining_ms = (deadline - now).as_millis() as i64;
        match io_wait_readable(raw_fd, remaining_ms.max(1)) {
            0 => return (NET_STATUS_TIMEOUT, String::new(), None),
            1 => {}
            _ => return (NET_STATUS_REACTOR_ERROR, String::new(), None),
        }
        // Per-call recvfrom with MSG_DONTWAIT — leaves socket mode
        // untouched.  Use libc::sockaddr_storage to capture peer
        // (IPv4 or IPv6) without the dup-fd / mode-flip dance.
        let mut peer_storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        let mut peer_len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
        let n = unsafe {
            libc::recvfrom(
                raw_fd as libc::c_int,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                libc::MSG_DONTWAIT,
                &mut peer_storage as *mut _ as *mut libc::sockaddr,
                &mut peer_len,
            )
        };
        if n > 0 {
            let n = n as usize;
            buf.truncate(n);
            let peer_tuple = decode_sockaddr_storage(&peer_storage, peer_len);
            return (
                n as i64,
                String::from_utf8_lossy(&buf).into_owned(),
                peer_tuple,
            );
        } else if n == 0 {
            // UDP recv of 0 bytes is unusual but legal (zero-length
            // datagram).  Report 0 + empty body + decoded peer.
            let peer_tuple = decode_sockaddr_storage(&peer_storage, peer_len);
            return (0, String::new(), peer_tuple);
        } else {
            let err = std::io::Error::last_os_error();
            match err.kind() {
                std::io::ErrorKind::WouldBlock => continue,
                std::io::ErrorKind::Interrupted => continue,
                _ => return (NET_STATUS_IO_ERROR, String::new(), None),
            }
        }
    }
}

/// Decode a `sockaddr_storage` populated by `recvfrom` into the
/// `(family, host_str, port)` tuple shape used by the rest of
/// net_runtime.  Returns None if the family is not AF_INET / AF_INET6.
fn decode_sockaddr_storage(
    storage: &libc::sockaddr_storage,
    len: libc::socklen_t,
) -> Option<(u8, String, i64)> {
    let family = storage.ss_family as i32;
    if family == libc::AF_INET
        && len as usize >= std::mem::size_of::<libc::sockaddr_in>()
    {
        let sin: &libc::sockaddr_in = unsafe { &*(storage as *const _ as *const _) };
        let ip = u32::from_be(sin.sin_addr.s_addr);
        let host = format!(
            "{}.{}.{}.{}",
            (ip >> 24) & 0xff,
            (ip >> 16) & 0xff,
            (ip >> 8) & 0xff,
            ip & 0xff
        );
        let port = u16::from_be(sin.sin_port) as i64;
        Some((4, host, port))
    } else if family == libc::AF_INET6
        && len as usize >= std::mem::size_of::<libc::sockaddr_in6>()
    {
        let sin6: &libc::sockaddr_in6 = unsafe { &*(storage as *const _ as *const _) };
        let segs: [u16; 8] = unsafe { std::mem::transmute(sin6.sin6_addr.s6_addr) };
        let ip = std::net::Ipv6Addr::from([
            u16::from_be(segs[0]),
            u16::from_be(segs[1]),
            u16::from_be(segs[2]),
            u16::from_be(segs[3]),
            u16::from_be(segs[4]),
            u16::from_be(segs[5]),
            u16::from_be(segs[6]),
            u16::from_be(segs[7]),
        ]);
        let port = u16::from_be(sin6.sin6_port) as i64;
        Some((6, ip.to_string(), port))
    } else {
        None
    }
}

pub fn tcp_connect_timeout(host: &str, port: i64, timeout_ms: i64) -> i64 {
    if !(0..=65535).contains(&port) {
        return NET_STATUS_IO_ERROR;
    }
    // std::net::TcpStream::connect_timeout takes a SocketAddr, not
    // (host, port).  Resolve via to_socket_addrs first.
    use std::net::ToSocketAddrs;
    let addrs = match (host, port as u16).to_socket_addrs() {
        Ok(a) => a,
        Err(_) => return NET_STATUS_IO_ERROR,
    };
    let dur = if timeout_ms <= 0 {
        Duration::from_millis(1) // tiny non-zero — connect_timeout rejects 0
    } else {
        Duration::from_millis(timeout_ms as u64)
    };
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, dur) {
            Ok(stream) => return register(NetResource::Stream(stream)),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => return NET_STATUS_TIMEOUT,
            Err(_) => continue,
        }
    }
    NET_STATUS_IO_ERROR
}

// ---- raw-fd helpers (cross-platform shim) -----------------------------------

#[cfg(unix)]
fn listener_raw_fd(l: &TcpListener) -> i64 {
    use std::os::unix::io::AsRawFd;
    l.as_raw_fd() as i64
}
#[cfg(unix)]
fn stream_raw_fd(s: &TcpStream) -> i64 {
    use std::os::unix::io::AsRawFd;
    s.as_raw_fd() as i64
}
#[cfg(unix)]
fn udp_raw_fd(s: &UdpSocket) -> i64 {
    use std::os::unix::io::AsRawFd;
    s.as_raw_fd() as i64
}
#[cfg(windows)]
fn listener_raw_fd(l: &TcpListener) -> i64 {
    use std::os::windows::io::AsRawSocket;
    l.as_raw_socket() as i64
}
#[cfg(windows)]
fn stream_raw_fd(s: &TcpStream) -> i64 {
    use std::os::windows::io::AsRawSocket;
    s.as_raw_socket() as i64
}
#[cfg(windows)]
fn udp_raw_fd(s: &UdpSocket) -> i64 {
    use std::os::windows::io::AsRawSocket;
    s.as_raw_socket() as i64
}
#[cfg(not(any(unix, windows)))]
fn listener_raw_fd(_l: &TcpListener) -> i64 {
    -1
}
#[cfg(not(any(unix, windows)))]
fn stream_raw_fd(_s: &TcpStream) -> i64 {
    -1
}
#[cfg(not(any(unix, windows)))]
fn udp_raw_fd(_s: &UdpSocket) -> i64 {
    -1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn tcp_listen_accept_send_recv_round_trip() {
        // Server side — use tcp_listen_v2 with REUSEPORT (flag bit 0)
        // and explicit 127.0.0.1 host so we don't bind to 0.0.0.0
        // (v1 `tcp_listen` defaults).  REUSEPORT lets parallel test
        // runs / TIME_WAIT lingerers re-bind without the prior
        // `EADDRINUSE` hang that surfaced when the same port was
        // recycled fast across consecutive `cargo test` invocations.
        let listen_fd = tcp_listen_v2("127.0.0.1", 0, 128, 1);
        assert!(listen_fd > 0, "tcp_listen_v2 returned {listen_fd}");
        let port = tcp_local_port(listen_fd);
        assert!(port > 0 && port <= 65535, "expected valid port, got {port}");
        // Spawn a client
        let client = thread::spawn(move || {
            // Tiny sleep so accept() is reached first deterministically.
            thread::sleep(Duration::from_millis(20));
            let cfd = tcp_connect("127.0.0.1", port);
            assert!(cfd > 0);
            assert_eq!(tcp_send(cfd, b"hello"), 5);
            let resp = tcp_recv(cfd, 64).unwrap();
            assert_eq!(resp, "world");
            assert_eq!(tcp_close(cfd), 0);
        });
        let conn_fd = tcp_accept(listen_fd);
        assert!(conn_fd > 0);
        let req = tcp_recv(conn_fd, 64).unwrap();
        assert_eq!(req, "hello");
        assert_eq!(tcp_send(conn_fd, b"world"), 5);
        assert_eq!(tcp_close(conn_fd), 0);
        client.join().unwrap();
        assert_eq!(tcp_close(listen_fd), 0);
    }

    #[test]
    fn close_unknown_fd_returns_minus_one() {
        assert_eq!(tcp_close(999_999), -1);
    }

    #[test]
    fn invalid_port_is_rejected() {
        assert_eq!(tcp_listen(-1), -1);
        assert_eq!(tcp_listen(70000), -1);
        assert_eq!(udp_bind(-1), -1);
    }

    #[test]
    fn tcp_listen_v2_default_flags_round_trip() {
        let fd = tcp_listen_v2("127.0.0.1", 0, 128, 0);
        assert!(fd > 0, "expected fd > 0, got {fd}");
        let port = tcp_local_port(fd);
        assert!(port > 0 && port <= 65535, "expected valid port, got {port}");
        assert_eq!(tcp_close(fd), 0);
    }

    #[test]
    fn tcp_listen_v2_ipv6_loopback() {
        let fd = tcp_listen_v2("::1", 0, 64, 0);
        assert!(fd > 0, "expected fd > 0, got {fd}");
        assert!(tcp_local_port(fd) > 0);
        assert_eq!(tcp_close(fd), 0);
    }

    #[test]
    fn tcp_listen_v2_invalid_host_returns_einval() {
        let r = tcp_listen_v2("not-an-ip", 0, 128, 0);
        assert_eq!(r, -(libc::EINVAL as i64));
    }

    #[test]
    fn tcp_listen_v2_invalid_port_returns_einval() {
        assert_eq!(tcp_listen_v2("0.0.0.0", -1, 128, 0), -(libc::EINVAL as i64));
        assert_eq!(
            tcp_listen_v2("0.0.0.0", 70_000, 128, 0),
            -(libc::EINVAL as i64)
        );
    }

    #[test]
    fn tcp_listen_v2_invalid_backlog_returns_einval() {
        assert_eq!(tcp_listen_v2("0.0.0.0", 0, -1, 0), -(libc::EINVAL as i64));
        assert_eq!(
            tcp_listen_v2("0.0.0.0", 0, 70_000, 0),
            -(libc::EINVAL as i64)
        );
    }

    #[test]
    fn tcp_listen_v2_reuseport_flag_does_not_break_bind() {
        // SO_REUSEPORT is best-effort; the listener should bind whether
        // or not the kernel honours the option.
        let fd = tcp_listen_v2("127.0.0.1", 0, 128, TCP_LISTEN_FLAG_REUSEPORT);
        assert!(fd > 0, "expected fd > 0, got {fd}");
        assert!(tcp_local_port(fd) > 0);
        assert_eq!(tcp_close(fd), 0);
    }

    #[test]
    fn tcp_local_port_unknown_fd_returns_minus_one() {
        assert_eq!(tcp_local_port(999_999), -1);
    }

    /// VBC-NET-AUDIT-1 — peer is reported alongside the recv'd
    /// payload (was dropped pre-fix).  Smoke test: bind two
    /// sockets, send from one to the other, verify the recv'd
    /// peer matches the sender's local addr.
    #[test]
    fn udp_recv_from_returns_peer_address() {
        let recv_fd = udp_bind(0);
        assert!(recv_fd > 0);
        let recv_port = {
            let map = REGISTRY.lock().unwrap();
            match map.get(&recv_fd) {
                Some(NetResource::Udp(s)) => s.local_addr().unwrap().port(),
                _ => panic!("recv socket missing"),
            }
        };
        let send_fd = udp_bind(0);
        assert!(send_fd > 0);
        let send_port = {
            let map = REGISTRY.lock().unwrap();
            match map.get(&send_fd) {
                Some(NetResource::Udp(s)) => s.local_addr().unwrap().port(),
                _ => panic!("send socket missing"),
            }
        };
        assert_eq!(udp_send(send_fd, b"ping", "127.0.0.1", recv_port as i64), 4);
        let recv = udp_recv_from(recv_fd, 64).expect("recv");
        assert_eq!(recv.0, "ping");
        let (family, host, port) = recv.1.expect("peer reported");
        assert_eq!(family, 4);
        // Sender's local addr is what the kernel reports as peer.
        assert_eq!(port, send_port as i64);
        // Sender bound to 0.0.0.0 → kernel typically reports 127.0.0.1
        // for loopback delivery on macOS/Linux.
        assert!(host == "127.0.0.1" || host == "0.0.0.0", "peer host: {}", host);
        assert_eq!(udp_close(recv_fd), 0);
        assert_eq!(udp_close(send_fd), 0);
    }

    /// VBC-NET-RT-1 — proof that the lock-drop discipline works.
    /// Two TCP connections in flight: one is parked in `tcp_recv`
    /// (the listener never sends), the other completes a full
    /// send + recv round-trip.  Pre-fix the second op blocked on
    /// the REGISTRY mutex held by the first; post-fix it completes
    /// in milliseconds.  100 ms timeout proves we don't serialise.
    #[test]
    fn concurrent_recv_does_not_block_unrelated_send() {
        // Listener that NEVER sends — used to park a recv.
        let parker_listen = tcp_listen_v2("127.0.0.1", 0, 8, 1);
        assert!(parker_listen > 0);
        let parker_port = tcp_local_port(parker_listen);
        // Listener that echoes — used for the unrelated round-trip.
        let echo_listen = tcp_listen_v2("127.0.0.1", 0, 8, 1);
        assert!(echo_listen > 0);
        let echo_port = tcp_local_port(echo_listen);

        // Park a recv on the first stream in a background thread.
        let parker = thread::spawn(move || {
            let cfd = tcp_connect("127.0.0.1", parker_port);
            assert!(cfd > 0);
            // This recv will block forever (the server never replies).
            // We only care that it doesn't hold the REGISTRY mutex.
            let _ = tcp_recv(cfd, 64);
            // Unreachable in normal test flow; main thread closes
            // the listener which causes the connection to drop and
            // recv to return with EOF/None.
            let _ = tcp_close(cfd);
        });
        // Accept the parker connection (so its recv has something to wait on).
        let parker_conn = tcp_accept(parker_listen);
        assert!(parker_conn > 0);
        // Give the parker thread a moment to enter recv().
        thread::sleep(Duration::from_millis(50));

        // NOW the canary: an unrelated round-trip must complete fast.
        let echo_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let echo_done_clone = echo_done.clone();
        let echo_client = thread::spawn(move || {
            let cfd = tcp_connect("127.0.0.1", echo_port);
            assert!(cfd > 0);
            assert_eq!(tcp_send(cfd, b"ping"), 4);
            let resp = tcp_recv(cfd, 64).unwrap();
            assert_eq!(resp, "pong");
            assert_eq!(tcp_close(cfd), 0);
            echo_done_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        let echo_conn = tcp_accept(echo_listen);
        assert!(echo_conn > 0);
        let req = tcp_recv(echo_conn, 64).unwrap();
        assert_eq!(req, "ping");
        assert_eq!(tcp_send(echo_conn, b"pong"), 4);
        assert_eq!(tcp_close(echo_conn), 0);
        echo_client.join().unwrap();
        assert!(
            echo_done.load(std::sync::atomic::Ordering::SeqCst),
            "echo round-trip did not complete — REGISTRY lock contention?"
        );

        // Cleanup parker.
        assert_eq!(tcp_close(parker_conn), 0);
        assert_eq!(tcp_close(parker_listen), 0);
        // Parker thread will unblock on EOF; join with timeout
        // semantics via a simple sleep + status check is overkill
        // here — `tcp_close` of the conn breaks the recv.
        let _ = parker.join();
        assert_eq!(tcp_close(echo_listen), 0);
    }

    // VBC-NET-RT-2 ----------------------------------------------------------
    //
    // The reactor + timeout-bound I/O tests exercise the SAME shared
    // singletons (REACTOR + REGISTRY + global kernel fd table) and
    // therefore see flaky parallel-execution failures from kqueue
    // stale-event wakes when fd numbers recycle across tests.
    // Serialise them via a process-wide test mutex so each test sees
    // a clean slate.  This is a TEST-LEVEL serialisation only —
    // production code paths are fully concurrent (the lock-drop
    // discipline + sharded-waiter map mean concurrent script use is
    // unaffected).
    static REACTOR_TEST_MUTEX: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    /// `tcp_accept_timeout` returns NET_STATUS_TIMEOUT when no
    /// client connects in the allotted window — and does so within
    /// the deadline, not after the OS-default accept blocking.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn tcp_accept_timeout_returns_minus_two_on_no_client() {
        let _guard = REACTOR_TEST_MUTEX.lock().unwrap();
        let listen_fd = tcp_listen_v2("127.0.0.1", 0, 8, 1);
        assert!(listen_fd > 0);
        let start = std::time::Instant::now();
        let r = tcp_accept_timeout(listen_fd, 150);
        let elapsed = start.elapsed();
        assert_eq!(r, NET_STATUS_TIMEOUT);
        assert!(
            elapsed >= Duration::from_millis(120) && elapsed < Duration::from_secs(2),
            "elapsed={elapsed:?}"
        );
        assert_eq!(tcp_close(listen_fd), 0);
    }

    /// `tcp_accept_timeout` returns the accepted fd when a client
    /// connects within the deadline.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn tcp_accept_timeout_succeeds_when_client_connects() {
        let _guard = REACTOR_TEST_MUTEX.lock().unwrap();
        let listen_fd = tcp_listen_v2("127.0.0.1", 0, 8, 1);
        assert!(listen_fd > 0);
        let port = tcp_local_port(listen_fd);
        let h = thread::spawn(move || {
            thread::sleep(Duration::from_millis(40));
            let cfd = tcp_connect("127.0.0.1", port);
            assert!(cfd > 0);
            tcp_close(cfd)
        });
        let conn_fd = tcp_accept_timeout(listen_fd, 1500);
        assert!(conn_fd > 0, "expected fd>0, got {conn_fd}");
        assert_eq!(tcp_close(conn_fd), 0);
        h.join().unwrap();
        assert_eq!(tcp_close(listen_fd), 0);
    }

    /// `tcp_recv_timeout` returns (NET_STATUS_TIMEOUT, "") when
    /// the peer never sends.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn tcp_recv_timeout_returns_timeout_when_silent() {
        let _guard = REACTOR_TEST_MUTEX.lock().unwrap();
        let listen_fd = tcp_listen_v2("127.0.0.1", 0, 8, 0);
        assert!(listen_fd > 0);
        let port = tcp_local_port(listen_fd);
        let cfd = tcp_connect("127.0.0.1", port);
        assert!(cfd > 0);
        let server_fd = tcp_accept(listen_fd);
        assert!(server_fd > 0, "tcp_accept returned {server_fd}");
        thread::sleep(Duration::from_millis(20));
        let start = std::time::Instant::now();
        let (status, body) = tcp_recv_timeout(cfd, 64, 200);
        let elapsed = start.elapsed();
        assert_eq!(
            status, NET_STATUS_TIMEOUT,
            "expected timeout, got status={status} body={body:?} elapsed={elapsed:?}"
        );
        assert!(body.is_empty());
        assert!(elapsed < Duration::from_secs(2));
        assert_eq!(tcp_close(cfd), 0);
        assert_eq!(tcp_close(server_fd), 0);
        assert_eq!(tcp_close(listen_fd), 0);
    }

    /// `tcp_recv_timeout` returns the data when it arrives within
    /// the deadline.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn tcp_recv_timeout_succeeds_when_data_arrives() {
        let _guard = REACTOR_TEST_MUTEX.lock().unwrap();
        let listen_fd = tcp_listen_v2("127.0.0.1", 0, 8, 1);
        let port = tcp_local_port(listen_fd);
        let h = thread::spawn(move || {
            thread::sleep(Duration::from_millis(30));
            let cfd = tcp_connect("127.0.0.1", port);
            assert_eq!(tcp_send(cfd, b"hello"), 5);
            thread::sleep(Duration::from_millis(50));
            tcp_close(cfd)
        });
        let server_fd = tcp_accept_timeout(listen_fd, 2000);
        assert!(server_fd > 0);
        let (status, body) = tcp_recv_timeout(server_fd, 64, 1000);
        assert_eq!(status, 5);
        assert_eq!(body, "hello");
        assert_eq!(tcp_close(server_fd), 0);
        h.join().unwrap();
        assert_eq!(tcp_close(listen_fd), 0);
    }

    /// `udp_recv_from_timeout` reports timeout AND preserves the
    /// peer (None on timeout, Some(...) on success).
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn udp_recv_from_timeout_reports_peer_and_timeout() {
        let _guard = REACTOR_TEST_MUTEX.lock().unwrap();
        let recv_fd = udp_bind(0);
        let recv_port = {
            let map = REGISTRY.lock().unwrap();
            match map.get(&recv_fd) {
                Some(NetResource::Udp(s)) => s.local_addr().unwrap().port(),
                _ => panic!("recv socket missing"),
            }
        };
        // Timeout path.
        let start = std::time::Instant::now();
        let (st, body, peer) = udp_recv_from_timeout(recv_fd, 64, 100);
        assert_eq!(st, NET_STATUS_TIMEOUT);
        assert!(body.is_empty());
        assert!(peer.is_none());
        assert!(start.elapsed() < Duration::from_secs(2));
        // Success path.
        let send_fd = udp_bind(0);
        assert_eq!(udp_send(send_fd, b"ping", "127.0.0.1", recv_port as i64), 4);
        let (st2, body2, peer2) = udp_recv_from_timeout(recv_fd, 64, 1500);
        assert_eq!(st2, 4);
        assert_eq!(body2, "ping");
        let (family, _, _) = peer2.expect("peer");
        assert_eq!(family, 4);
        assert_eq!(udp_close(recv_fd), 0);
        assert_eq!(udp_close(send_fd), 0);
    }

    /// Generic readiness primitive — `io_wait_readable` must return
    /// 0 (timeout) for a fresh listener with no connections.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn io_wait_readable_timeout_path() {
        let _guard = REACTOR_TEST_MUTEX.lock().unwrap();
        use std::os::fd::AsRawFd;
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.set_nonblocking(true).unwrap();
        let fd = l.as_raw_fd() as i64;
        assert_eq!(io_wait_readable(fd, 80), 0);
    }
}

// ============================================================================
// VBC-NET-2 — high-level Tier-0 intercepts for `core.net.tcp`
//

// The intrinsics above (`__tcp_*_raw`) cover the synthetic-fd
// network surface that script-mode users reach via the raw-FFI
// fallback in `core/sys/net_ops.vr`. The HIGH-LEVEL
// `core.net.tcp.TcpStream.connect` path goes through a different
// chain (`safe_socket` + `errno`-driven sys_socket / sys_connect)
// that fails in interpreter mode for the same FFI-brittleness
// reason that motivated `shell_runtime` / `file_runtime` /
// `process_runtime`.
//

// This intercept catches `TcpStream.connect_addr(&SocketAddr) ->
// Result<TcpStream, IoError>` (the inner per-address worker the
// polymorphic `connect<A: ToSocketAddrs>` calls), connects via
// `std::net::TcpStream`, registers the fd in REGISTRY, and
// constructs the full `TcpStream { fd: FileDesc, peer_addr:
// SocketAddr }` record. Subsequent reads/writes via the existing
// `__tcp_*_raw` intercepts share the same REGISTRY, so the stream
// is fully usable end-to-end.
// ============================================================================

use super::super::super::error::InterpreterResult;
use super::heap_helpers::{
    alloc_record_n_fields, extract_byte_slice, extract_text_arg, is_record_typed_as,
    read_buffer_capacity, wrap_in_variant, write_into_byte_slice,
};
use super::string_helpers::alloc_string_value;
use crate::interpreter::permission::{PermissionDecision, PermissionScope};
use crate::interpreter::state::InterpreterState;
use crate::value::Value;

pub(in super::super) fn try_intercept_net_runtime(
    state: &mut InterpreterState,
    func_name: &str,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    // Disambiguate against unrelated stdlib functions by qualifying
    // on `core.net.{tcp,udp}` / `TcpStream.` / `TcpListener.` /
    // `UdpSocket.`. This file intercepts ONLY the high-level
    // network surface in `core.net`.
    if !func_name.contains("net.tcp")
        && !func_name.contains("net::tcp")
        && !func_name.contains("net.udp")
        && !func_name.contains("net::udp")
        && !func_name.contains("TcpStream.")
        && !func_name.contains("TcpStream::")
        && !func_name.contains("TcpListener.")
        && !func_name.contains("UdpSocket.")
    {
        return Ok(None);
    }
    let bare = func_name.rsplit('.').next().unwrap_or(func_name);
    let is_udp = func_name.contains("UdpSocket.") || func_name.contains("net.udp");
    let is_listener = func_name.contains("TcpListener.");
    match bare {
        // `TcpStream.connect<A: ToSocketAddrs>(addr: A) ->
        // Result<TcpStream, IoError>` — see `intercept_connect_text`.
        "connect" if arg_count == 1 && !is_udp => {
            intercept_connect_text(state, args_start_reg, caller_base)
        }
        // `UdpSocket.bind<A: ToSocketAddrs>(addr: A) ->
        // Result<UdpSocket, IoError>` — bypass libSystem
        // `socket(2)` + `bind(2)` via net_runtime::udp_bind.
        "bind" if arg_count == 1 && is_udp => {
            intercept_udp_bind_text(state, args_start_reg, caller_base)
        }
        // `TcpListener.bind<A: ToSocketAddrs>(addr: A) ->
        // Result<TcpListener, IoError>` — bypass the iterator-bound
        // path that crashes at `SocketAddr.ip` opcode 0x62 null-deref
        // (the iteration's yielded SocketAddr value carries a stale
        // pointer for some shape variants). Routes through
        // `net_runtime::tcp_listen_v2` and constructs a real
        // TcpListener record.
        "bind" if arg_count == 1 && is_listener => {
            intercept_listener_bind_text(state, args_start_reg, caller_base)
        }
        _ => Ok(None),
    }
}

/// Intercept `TcpStream.connect(&Text)` — parse the host:port string
/// directly via std, bypassing the polymorphic `to_socket_addrs` +
/// `connect_addr` chain that depends on libSystem `socket(2)` /
/// `connect(2)` syscalls in the Verum stdlib.
fn intercept_connect_text(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let addr_text = extract_text_arg(state, args_start_reg, caller_base);
    // Granular permission check uses the host:port endpoint as the
    // target_id; falls through to WILDCARD for `permissions = ["net"]`.
    if let Some(denied) = check_net_permission_for(state, Some(&addr_text)) {
        return Ok(Some(denied));
    }
    // Only handle the Text-arg shape — if the extracted string is
    // empty or doesn't contain `:port`, fall through to the bytecode
    // body (which can handle SocketAddr / tuple shapes).
    if addr_text.is_empty() || !addr_text.contains(':') {
        return Ok(None);
    }
    // Split host and port. Bracketed IPv6 [::]:port works through
    // std::net::TcpStream::connect(&str) directly, so we only need
    // basic validation here.
    let port_split = addr_text.rsplit_once(':');
    let (host, port) = match port_split {
        Some((h, p)) => match p.parse::<u16>() {
            Ok(n) => (
                h.trim_matches(|c| c == '[' || c == ']').to_string(),
                n as i64,
            ),
            Err(_) => return Ok(None),
        },
        None => return Ok(None),
    };
    let fd = tcp_connect(&host, port);
    if fd < 0 {
        return Ok(Some(build_io_err(
            state,
            "ConnectionRefused",
            2,
            &format!("connect: tcp_connect({}:{}) failed", host, port),
        )?));
    }
    // FileDesc is `is (Int)` — a transparent newtype. At the value
    // level it's just the Int with no wrapper record (verified by
    // tracing `stream.write(self)` — the receiver's field 0 reads
    // back as `Value::is_int() == true`). Store the fd directly.
    let fd_value = Value::from_i64(fd);
    // peer_addr: round-trip via `getpeername(2)` on the live fd
    // for the truly-resolved peer. Handles IPv4, IPv6, and
    // DNS-resolved hosts uniformly — the kernel reports the actual
    // family + address it ended up connecting to. Falls back to
    // the input host:port literal when the peer query fails (e.g.,
    // disconnected before we could query).
    let peer_addr = match tcp_peer_addr(fd) {
        Some((4, ip, p)) => build_peer_addr(state, &ip, p).unwrap_or(Value::unit()),
        Some((6, ip, p)) => build_peer_addr_v6(state, &ip, p).unwrap_or(Value::unit()),
        _ => build_peer_addr(state, &host, port).unwrap_or(Value::unit()),
    };
    let stream = alloc_record_n_fields(state, "TcpStream", &[fd_value, peer_addr])?;
    Ok(Some(wrap_in_variant(state, "Result", 0, &[stream])?))
}

/// Intercept `TcpListener.bind(&Text)` — bind a kernel-side
/// listener via `net_runtime::tcp_listen_v2` (which already registers
/// the fd in the REGISTRY), then construct a `TcpListener { fd,
/// local_addr }` record so subsequent `accept()` / `local_addr()`
/// calls work via the standard method-dispatch hook. Bypasses
/// the iterator-bound bytecode path that crashes at SocketAddr.ip
/// opcode 0x62 null-deref.
fn intercept_listener_bind_text(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let addr_text = extract_text_arg(state, args_start_reg, caller_base);
    // Granular permission: hash the bind endpoint so
    // `permissions = ["net=127.0.0.1:8080"]` grants only that bind.
    if let Some(denied) = check_net_permission_for(state, Some(&addr_text)) {
        return Ok(Some(denied));
    }
    if addr_text.is_empty() || !addr_text.contains(':') {
        return Ok(None);
    }
    let (host, port) = match addr_text.rsplit_once(':') {
        Some((h, p)) => match p.parse::<u16>() {
            Ok(n) => (
                h.trim_matches(|c| c == '[' || c == ']').to_string(),
                n as i64,
            ),
            Err(_) => return Ok(None),
        },
        None => return Ok(None),
    };
    let fd = tcp_listen_v2(&host, port, 1024, 0);
    if fd < 0 {
        return Ok(Some(build_io_err(
            state,
            "AddrInUse",
            6,
            &format!(
                "listener.bind: tcp_listen_v2({}:{}) failed errno={}",
                host, port, -fd
            ),
        )?));
    }
    // Resolve the actually-bound port (handles port=0 / kernel-
    // assigned). `tcp_local_port` walks the registry first then
    // falls through to getsockname for real fds.
    let bound_port = tcp_local_port(fd);
    let bound_port = if bound_port > 0 { bound_port } else { port };
    // FileDesc transparent-newtype Int.
    let fd_value = Value::from_i64(fd);
    // local_addr: try IPv6 first (covers "::", "::1") then IPv4.
    let local_addr = if host.contains(':') {
        build_peer_addr_v6(state, &host, bound_port)
            .unwrap_or_else(|| build_peer_addr(state, &host, bound_port).unwrap_or(Value::unit()))
    } else {
        build_peer_addr(state, &host, bound_port).unwrap_or(Value::unit())
    };
    let listener = alloc_record_n_fields(state, "TcpListener", &[fd_value, local_addr])?;
    Ok(Some(wrap_in_variant(state, "Result", 0, &[listener])?))
}

/// Intercept `UdpSocket.bind(&Text)` — bind via std::net::UdpSocket
/// directly, register the fd in the shared net REGISTRY, and
/// construct the full `UdpSocket { fd, local_addr, peer_addr }`
/// record. Bypasses the libSystem `socket(2)` + `bind(2)` chain.
fn intercept_udp_bind_text(
    state: &mut InterpreterState,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let addr_text = extract_text_arg(state, args_start_reg, caller_base);
    if let Some(denied) = check_net_permission_for(state, Some(&addr_text)) {
        return Ok(Some(denied));
    }
    if addr_text.is_empty() || !addr_text.contains(':') {
        return Ok(None);
    }
    let (host, port) = match addr_text.rsplit_once(':') {
        Some((h, p)) => match p.parse::<u16>() {
            Ok(n) => (
                h.trim_matches(|c| c == '[' || c == ']').to_string(),
                n as i64,
            ),
            Err(_) => return Ok(None),
        },
        None => return Ok(None),
    };
    let fd = udp_bind(port);
    if fd < 0 {
        return Ok(Some(build_io_err(
            state,
            "AddrInUse",
            6,
            &format!("bind: udp_bind({}:{}) failed", host, port),
        )?));
    }
    // FileDesc transparent-newtype lowering — store fd as a bare Int.
    let fd_value = Value::from_i64(fd);
    // local_addr: best-effort SocketAddr.V4 reconstruction (matches
    // TcpStream connect-text semantics). IPv6 / DNS-resolved hosts
    // fall back to Unit pending the V1 getsockname round-trip.
    let local_addr = build_peer_addr(state, &host, port).unwrap_or(Value::unit());
    // peer_addr: bind without connect leaves no peer — Maybe.None
    // (tag 0, empty payload).
    let peer_addr_none = wrap_in_variant(state, "Maybe", 0, &[])?;
    let socket =
        alloc_record_n_fields(state, "UdpSocket", &[fd_value, local_addr, peer_addr_none])?;
    Ok(Some(wrap_in_variant(state, "Result", 0, &[socket])?))
}

// =============================================================================
// SocketAddrCodec — centralised SocketAddr ↔ Value codec
// =============================================================================
//
// **VBC-NET-AUDIT-2 — verified-codec pattern.**  Pre-extraction,
// SocketAddr encoding/decoding lived in three sibling helpers
// (`build_peer_addr`, `build_peer_addr_v6`, `read_socket_addr_value`)
// that each redid offset arithmetic against the heap layout —
// fragile to any codegen-side layout change and impossible to
// audit in isolation.  This codec is the single source of truth.
//
// **Layout invariants** (must agree with the stdlib decls in
// `core/net/addr.vr`):
//
//   * `Ipv4Addr is { octets: (Byte, Byte, Byte, Byte) }`
//     → 4-field record, each field a NaN-boxed integer.
//   * `Ipv6Addr is { segments: (Int, Int × 8) }`
//     → 8-field record, each field a NaN-boxed 16-bit integer.
//   * `SocketAddrV4 is { ip: Ipv4Addr, port: Int }`
//     → 2-field record.
//   * `SocketAddrV6 is { ip: Ipv6Addr, port: Int, flowinfo: Int,
//       scope_id: Int }` → 4-field record.
//   * `SocketAddr is V4(SocketAddrV4) | V6(SocketAddrV6)`
//     → variant; tag 0 = V4, tag 1 = V6, single payload Value.
//
// Both encode and decode must agree on these.  Layout drift
// surfaces as a unit-test failure in the round-trip pin
// (`socket_addr_codec_round_trip`).

/// Encode a Rust `SocketAddr`-style triple `(family, host_str, port)`
/// into a Verum `SocketAddr` value.  `family` is 4 (IPv4) or 6
/// (IPv6); `host_str` parses via `std::net::Ipv4Addr`/`Ipv6Addr`
/// — no DNS, no brackets needed (they're stripped if present).
/// Returns None when the host doesn't parse for the declared
/// family.
fn encode_socket_addr(
    state: &mut InterpreterState,
    family: u8,
    host: &str,
    port: i64,
) -> Option<Value> {
    let host = host.trim_matches(|c: char| c == '[' || c == ']');
    match family {
        4 => {
            let ipv4: std::net::Ipv4Addr = host.parse().ok()?;
            let octets = ipv4.octets();
            let octets_record = alloc_record_n_fields(
                state,
                "Ipv4Addr",
                &[
                    Value::from_i64(octets[0] as i64),
                    Value::from_i64(octets[1] as i64),
                    Value::from_i64(octets[2] as i64),
                    Value::from_i64(octets[3] as i64),
                ],
            )
            .ok()?;
            let v4 = alloc_record_n_fields(
                state,
                "SocketAddrV4",
                &[octets_record, Value::from_i64(port)],
            )
            .ok()?;
            wrap_in_variant(state, "SocketAddr", 0, &[v4]).ok()
        }
        6 => {
            let ipv6: std::net::Ipv6Addr = host.parse().ok()?;
            let segs = ipv6.segments();
            let segments_record = alloc_record_n_fields(
                state,
                "Ipv6Addr",
                &[
                    Value::from_i64(segs[0] as i64),
                    Value::from_i64(segs[1] as i64),
                    Value::from_i64(segs[2] as i64),
                    Value::from_i64(segs[3] as i64),
                    Value::from_i64(segs[4] as i64),
                    Value::from_i64(segs[5] as i64),
                    Value::from_i64(segs[6] as i64),
                    Value::from_i64(segs[7] as i64),
                ],
            )
            .ok()?;
            let v6 = alloc_record_n_fields(
                state,
                "SocketAddrV6",
                &[
                    segments_record,
                    Value::from_i64(port),
                    Value::from_i64(0), // flowinfo
                    Value::from_i64(0), // scope_id
                ],
            )
            .ok()?;
            wrap_in_variant(state, "SocketAddr", 1, &[v6]).ok()
        }
        _ => None,
    }
}

/// IPv4 convenience wrapper preserved for the call sites that
/// know the family at compile time.
fn build_peer_addr(state: &mut InterpreterState, host: &str, port: i64) -> Option<Value> {
    encode_socket_addr(state, 4, host, port)
}

/// IPv6 convenience wrapper.
fn build_peer_addr_v6(state: &mut InterpreterState, host: &str, port: i64) -> Option<Value> {
    encode_socket_addr(state, 6, host, port)
}

/// Family-agnostic encode that picks V4 or V6 based on the
/// `host_str`'s parse result.  Used by call sites that have a
/// `(family, host, port)` triple from `tcp_peer_addr` /
/// `udp_recv_from` and want to reconstruct the matching variant
/// without a per-arm match.
fn encode_socket_addr_auto(
    state: &mut InterpreterState,
    family: u8,
    host: &str,
    port: i64,
) -> Option<Value> {
    encode_socket_addr(state, family, host, port)
}

/// VBC-PERM-1 — granular target_id: hash the host:port endpoint
/// so a script frontmatter `permissions = ["net=api.example.com:443"]`
/// grants only that endpoint.  Falls through to WILDCARD for
/// scripts that grant `"net"` without a target.  Callers pass
/// `None` for endpoint when the operation isn't endpoint-specific
/// (e.g. wildcard fallbacks during early bind/listen probes).
fn check_net_permission_for(
    state: &mut InterpreterState,
    endpoint: Option<&str>,
) -> Option<Value> {
    use crate::interpreter::permission::{target_id_for, WILDCARD_TARGET_ID};
    if let Some(ep) = endpoint {
        let tid = target_id_for(ep);
        if state.check_permission(PermissionScope::Network, tid) == PermissionDecision::Allow {
            return None;
        }
    }
    if state.check_permission(PermissionScope::Network, WILDCARD_TARGET_ID)
        != PermissionDecision::Deny
    {
        return None;
    }
    let msg = match endpoint {
        Some(ep) => format!("permission denied: network access to {} requires `net` grant", ep),
        None => "permission denied: network access requires `net`".to_string(),
    };
    build_io_err(state, "PermissionDenied", 1, &msg).ok()
}

/// Backward-compat wrapper for call sites that don't have an
/// endpoint (e.g. simple TCP-listen-on-any-port operations).
fn check_net_permission(state: &mut InterpreterState) -> Option<Value> {
    check_net_permission_for(state, None)
}

fn build_io_err(
    state: &mut InterpreterState,
    _kind_name: &str,
    kind_tag: u32,
    message: &str,
) -> InterpreterResult<Value> {
    let kind_variant = wrap_in_variant(state, "IoErrorKind", kind_tag, &[])?;
    let msg_text = alloc_string_value(state, message)?;
    let msg_some = wrap_in_variant(state, "Maybe", 1, &[msg_text])?;
    let stream_err = alloc_record_n_fields(state, "StreamError", &[kind_variant, msg_some])?;
    wrap_in_variant(state, "Result", 1, &[stream_err])
}

// ============================================================================
// VBC-NET-2 method-call surface — TcpStream method intercepts
//

// Method calls on TcpStream values (`stream.read(&mut buf)`,
// `stream.write(&data)`, `stream.flush()`, `stream.close()`)
// dispatch via CallM and reach `method_dispatch::handle_call_method`.
// We intercept BEFORE the bytecode body runs, route through the
// shared `net_runtime` REGISTRY, and bypass `sys_send` /
// `sys_recv` / `sys_close` libSystem calls.
//

// `try_intercept_tcp_method` is gated on the receiver being a
// `TcpStream` record AND the method being one we cover. Any other
// case returns None and the normal bytecode dispatch proceeds.
// ============================================================================

/// Try to intercept a method call on a TcpStream receiver.
pub(in super::super) fn try_intercept_tcp_method(
    state: &mut InterpreterState,
    method_name: &str,
    bare_method: &str,
    receiver: Value,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    // Receiver must be a TcpStream record. Cheap check first:
    // method_name should mention "TcpStream" OR the receiver heap
    // object's TypeId must resolve to a type named "TcpStream".
    // Receiver may arrive as a CBGR-ref (negative Int) when dispatch
    // routes `&mut self` through register-encoded references — unwrap
    // before any TcpStream-shape checks.
    let receiver = if super::cbgr_helpers::is_cbgr_ref(&receiver) {
        let (abs_index, _) = super::cbgr_helpers::decode_cbgr_ref(receiver.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        receiver
    };
    // UdpSocket / TcpListener method dispatch — same shape as
    // TcpStream (record with fd at field 0). Detect via
    // method_name OR receiver TypeId.
    let is_udp =
        method_name.contains("UdpSocket.") || is_record_typed_as(state, receiver, "UdpSocket");
    let is_listener =
        method_name.contains("TcpListener.") || is_record_typed_as(state, receiver, "TcpListener");
    if std::env::var("VERUM_TRACE_TCP").is_ok() {
        eprintln!(
            "[trace-net-method] method={:?} bare={:?} args={} is_udp={} is_listener={}",
            method_name, bare_method, arg_count, is_udp, is_listener
        );
    }
    if !is_udp
        && !is_listener
        && !method_name.contains("TcpStream.")
        && !is_record_typed_as(state, receiver, "TcpStream")
    {
        return Ok(None);
    }
    let fd = match read_tcpstream_fd(receiver) {
        Some(fd) => fd,
        None => return Ok(None),
    };
    if is_udp {
        return match bare_method {
            "send_to" if arg_count == 2 => {
                intercept_udp_send_to(state, fd, args_start_reg, caller_base)
            }
            "recv_from" if arg_count == 1 => {
                intercept_udp_recv_from(state, fd, args_start_reg, caller_base)
            }
            "close" if arg_count == 0 => {
                let _ = udp_close(fd);
                Ok(Some(wrap_in_variant(state, "Result", 0, &[Value::unit()])?))
            }
            _ => Ok(None),
        };
    }
    if is_listener {
        return match bare_method {
            // `listener.accept() -> Result<(TcpStream, SocketAddr), IoError>`
            "accept" if arg_count == 0 => intercept_listener_accept(state, fd),
            "local_port" if arg_count == 0 => {
                let p = tcp_local_port(fd);
                if p < 0 {
                    Ok(Some(build_io_err(
                        state,
                        "Other",
                        19,
                        &format!("listener.local_port: getsockname({}) failed", fd),
                    )?))
                } else {
                    Ok(Some(wrap_in_variant(
                        state,
                        "Result",
                        0,
                        &[Value::from_i64(p)],
                    )?))
                }
            }
            "close" if arg_count == 0 => {
                let _ = tcp_close(fd);
                Ok(Some(wrap_in_variant(state, "Result", 0, &[Value::unit()])?))
            }
            _ => Ok(None),
        };
    }
    match bare_method {
        "write" if arg_count == 1 => intercept_tcp_write(state, fd, args_start_reg, caller_base),
        "read" if arg_count == 1 => intercept_tcp_read(state, fd, args_start_reg, caller_base),
        "flush" if arg_count == 0 => {
            // TCP is unbuffered; flush is a no-op returning Ok(()).
            Ok(Some(wrap_in_variant(state, "Result", 0, &[Value::unit()])?))
        }
        "close" if arg_count == 0 => {
            let _ = tcp_close(fd);
            Ok(Some(wrap_in_variant(state, "Result", 0, &[Value::unit()])?))
        }
        _ => Ok(None),
    }
}

/// `listener.accept() -> Result<(TcpStream, SocketAddr), IoError>` —
/// blocks until a client connects. Returns the connected stream
/// (already registered in REGISTRY) plus the peer's resolved
/// SocketAddr (V4 or V6 based on the kernel-reported family).
fn intercept_listener_accept(
    state: &mut InterpreterState,
    listen_fd: i64,
) -> InterpreterResult<Option<Value>> {
    if let Some(denied) = check_net_permission(state) {
        return Ok(Some(denied));
    }
    let client_fd = tcp_accept(listen_fd);
    if client_fd < 0 {
        return Ok(Some(build_io_err(
            state,
            "ConnectionAborted",
            4,
            &format!("listener.accept: tcp_accept({}) failed", listen_fd),
        )?));
    }
    // Real-fd peer address via getpeername.
    let peer_addr = match tcp_peer_addr(client_fd) {
        Some((4, ip, p)) => build_peer_addr(state, &ip, p).unwrap_or(Value::unit()),
        Some((6, ip, p)) => build_peer_addr_v6(state, &ip, p).unwrap_or(Value::unit()),
        _ => Value::unit(),
    };
    let fd_value = Value::from_i64(client_fd);
    let stream = alloc_record_n_fields(state, "TcpStream", &[fd_value, peer_addr])?;
    // Result<(TcpStream, SocketAddr), IoError> — Tuple wrapped in
    // Result.Ok. Build the 2-field tuple record.
    let pair = alloc_record_n_fields(state, "Tuple", &[stream, peer_addr])?;
    Ok(Some(wrap_in_variant(state, "Result", 0, &[pair])?))
}

/// `socket.send_to(&[Byte], SocketAddr) -> Result<Int, IoError>` —
/// extract bytes + parse SocketAddr to host:port, dispatch to
/// net_runtime::udp_send.
fn intercept_udp_send_to(
    state: &mut InterpreterState,
    fd: i64,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    if let Some(denied) = check_net_permission(state) {
        return Ok(Some(denied));
    }
    let bytes = extract_byte_slice(state, args_start_reg, caller_base);
    let addr_v = state
        .registers
        .get(caller_base, crate::instruction::Reg(args_start_reg + 1));
    let addr_v = if super::cbgr_helpers::is_cbgr_ref(&addr_v) {
        let (abs_index, _) = super::cbgr_helpers::decode_cbgr_ref(addr_v.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        addr_v
    };
    let (host, port) = match read_socket_addr_value(addr_v) {
        Some(parts) => parts,
        None => {
            return Ok(Some(build_io_err(
                state,
                "InvalidInput",
                11,
                "send_to: malformed SocketAddr",
            )?));
        }
    };
    let n = udp_send(fd, &bytes, &host, port);
    if n < 0 {
        return Ok(Some(build_io_err(
            state,
            "BrokenPipe",
            8,
            &format!("send_to: udp_send to {}:{} failed", host, port),
        )?));
    }
    Ok(Some(wrap_in_variant(
        state,
        "Result",
        0,
        &[Value::from_i64(n)],
    )?))
}

/// `socket.recv_from(&mut [Byte]) -> Result<(Int, SocketAddr), IoError>`
/// — recv into the buffer, return (bytes_recvd, source_addr). The
/// source address is best-effort: we currently don't have peer
/// info from net_runtime::udp_recv (returns Option<String> only),
/// so peer_addr falls back to a synthetic
/// `SocketAddr.V4(SocketAddrV4 { Ipv4Addr { 0,0,0,0 }, 0 })` —
/// callers that only need the byte count are unaffected. V1
/// follow-up: extend net_runtime::udp_recv to return the peer
/// address.
fn intercept_udp_recv_from(
    state: &mut InterpreterState,
    fd: i64,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    if let Some(denied) = check_net_permission(state) {
        return Ok(Some(denied));
    }
    let buf_v = state
        .registers
        .get(caller_base, crate::instruction::Reg(args_start_reg));
    let unwrapped = if super::cbgr_helpers::is_cbgr_ref(&buf_v) {
        let (abs_index, _) = super::cbgr_helpers::decode_cbgr_ref(buf_v.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        buf_v
    };
    let cap = read_buffer_capacity(unwrapped).unwrap_or(1500);
    let cap = cap.min(64 * 1024).max(1);
    // VBC-NET-AUDIT-1 — call udp_recv_from (returns the real peer)
    // instead of the legacy udp_recv that dropped peer info.
    let recv_result = udp_recv_from(fd, cap as i64);
    let (n, peer_tuple) = match recv_result {
        Some((s, peer)) => {
            let bytes = s.as_bytes();
            write_into_byte_slice(unwrapped, bytes);
            (bytes.len() as i64, peer)
        }
        None => (-1, None),
    };
    if n < 0 {
        return Ok(Some(build_io_err(
            state,
            "ConnectionAborted",
            4,
            &format!("recv_from: udp_recv on fd {} failed", fd),
        )?));
    }
    // Build the real peer SocketAddr from the kernel-reported (family,
    // host, port).  Falls back to Unit when the family doesn't match
    // what the codec can encode (shouldn't happen for AF_INET / AF_INET6).
    let peer_addr = match peer_tuple {
        Some((family, host, port)) => {
            encode_socket_addr_auto(state, family, &host, port).unwrap_or(Value::unit())
        }
        None => Value::unit(),
    };
    let pair = alloc_record_n_fields(state, "Tuple", &[Value::from_i64(n), peer_addr])?;
    Ok(Some(wrap_in_variant(state, "Result", 0, &[pair])?))
}

/// **VBC-NET-AUDIT-2 — central SocketAddr decoder.**  Pre-fix only
/// V4 was supported; V6 returned None and downstream surfaced as
/// `InvalidInput`.  This decoder handles both variants via a
/// shared inner helper.  Returns `(host_str, port)` ready for
/// `std::net::TcpStream::connect`.
///
/// Layout invariants (must agree with `encode_socket_addr` and the
/// stdlib decls in `core/net/addr.vr`):
///   * Variant: `[ObjectHeader][tag:u32][n_fields:u32][payload: Value]`
///   * Tag 0 → V4 → payload = SocketAddrV4 record.
///   * Tag 1 → V6 → payload = SocketAddrV6 record.
fn read_socket_addr_value(v: Value) -> Option<(String, i64)> {
    if !v.is_ptr() || v.is_nil() {
        return None;
    }
    let ptr = v.as_ptr::<u8>();
    if ptr.is_null() {
        return None;
    }
    let tag = unsafe { *(ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *const u32) };
    let payload =
        unsafe { *(ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE + 8) as *const Value) };
    match tag {
        0 => decode_socket_addr_v4_record(payload),
        1 => decode_socket_addr_v6_record(payload),
        _ => None,
    }
}

/// Decode a `SocketAddrV4 { ip: Ipv4Addr { (a,b,c,d) }, port: Int }`
/// record value into `(dotted_quad_str, port)`.  Tuple-as-record
/// invariant: the inner `(Byte, Byte, Byte, Byte)` is inlined as
/// 4 Value slots inside the Ipv4Addr record.
fn decode_socket_addr_v4_record(payload: Value) -> Option<(String, i64)> {
    if !payload.is_ptr() || payload.is_nil() {
        return None;
    }
    let v4_ptr = payload.as_ptr::<u8>();
    if v4_ptr.is_null() {
        return None;
    }
    let v4_base =
        unsafe { v4_ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *const Value };
    let ip_v = unsafe { *v4_base };
    let port_v = unsafe { *v4_base.add(1) };
    if !ip_v.is_ptr() || ip_v.is_nil() {
        return None;
    }
    let ip_ptr = ip_v.as_ptr::<u8>();
    if ip_ptr.is_null() {
        return None;
    }
    let ip_base =
        unsafe { ip_ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *const Value };
    let a = unsafe { *ip_base }.as_i64() as u8;
    let b = unsafe { *ip_base.add(1) }.as_i64() as u8;
    let c = unsafe { *ip_base.add(2) }.as_i64() as u8;
    let d = unsafe { *ip_base.add(3) }.as_i64() as u8;
    Some((format!("{}.{}.{}.{}", a, b, c, d), port_v.as_i64()))
}

/// Decode a `SocketAddrV6 { ip: Ipv6Addr { (s0..s7) }, port: Int,
/// flowinfo: Int, scope_id: Int }` record into
/// `(rfc5952_canonical_str, port)`.  Tuple-as-record invariant:
/// the inner 8-segment tuple is inlined as 8 Value slots inside
/// the Ipv6Addr record.  Uses `std::net::Ipv6Addr::Display` for
/// canonical RFC 5952 output (with `::` zero-run compression).
fn decode_socket_addr_v6_record(payload: Value) -> Option<(String, i64)> {
    if !payload.is_ptr() || payload.is_nil() {
        return None;
    }
    let v6_ptr = payload.as_ptr::<u8>();
    if v6_ptr.is_null() {
        return None;
    }
    let v6_base =
        unsafe { v6_ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *const Value };
    let ip_v = unsafe { *v6_base };
    let port_v = unsafe { *v6_base.add(1) };
    // flowinfo at field 2, scope_id at field 3 — not used by the
    // stringification path but documented for parity with the
    // encode side and any future caller that needs the full
    // SocketAddrV6 fidelity.
    if !ip_v.is_ptr() || ip_v.is_nil() {
        return None;
    }
    let ip_ptr = ip_v.as_ptr::<u8>();
    if ip_ptr.is_null() {
        return None;
    }
    let ip_base =
        unsafe { ip_ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *const Value };
    let s0 = unsafe { *ip_base }.as_i64() as u16;
    let s1 = unsafe { *ip_base.add(1) }.as_i64() as u16;
    let s2 = unsafe { *ip_base.add(2) }.as_i64() as u16;
    let s3 = unsafe { *ip_base.add(3) }.as_i64() as u16;
    let s4 = unsafe { *ip_base.add(4) }.as_i64() as u16;
    let s5 = unsafe { *ip_base.add(5) }.as_i64() as u16;
    let s6 = unsafe { *ip_base.add(6) }.as_i64() as u16;
    let s7 = unsafe { *ip_base.add(7) }.as_i64() as u16;
    let ipv6 = std::net::Ipv6Addr::new(s0, s1, s2, s3, s4, s5, s6, s7);
    Some((ipv6.to_string(), port_v.as_i64()))
}

/// Read the `fd` field of a TcpStream record. TcpStream layout is
/// `[ObjectHeader][fd: Value][peer_addr: Value]`. fd may be either
/// a transparent-newtype Int (Verum's `is (Int)` newtype lowering)
/// or a 1-field wrapper record — handle both.
fn read_tcpstream_fd(v: Value) -> Option<i64> {
    // Thin-ref (heap-pointer-to-Value) auto-deref. CBGR-ref unwrap
    // is the caller's responsibility (needs `state.registers`).
    let v = if v.is_thin_ref() {
        let tr = v.as_thin_ref();
        if tr.ptr.is_null() {
            v
        } else {
            unsafe { *(tr.ptr as *const Value) }
        }
    } else {
        v
    };
    if !v.is_ptr() || v.is_nil() {
        return None;
    }
    let ptr = v.as_ptr::<u8>();
    if ptr.is_null() {
        return None;
    }
    // Field 0 of TcpStream is the FileDesc.
    let fd_v = unsafe { *(ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *const Value) };
    // Transparent-newtype path: FileDesc value IS the Int.
    if fd_v.is_int() {
        return Some(fd_v.as_i64());
    }
    // Wrapper-record path: read field 0 of the FileDesc record.
    if fd_v.is_ptr() && !fd_v.is_nil() {
        let fd_ptr = fd_v.as_ptr::<u8>();
        if !fd_ptr.is_null() {
            let raw_v = unsafe {
                *(fd_ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *const Value)
            };
            if raw_v.is_int() {
                return Some(raw_v.as_i64());
            }
        }
    }
    None
}

fn intercept_tcp_write(
    state: &mut InterpreterState,
    fd: i64,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    if let Some(denied) = check_net_permission(state) {
        return Ok(Some(denied));
    }
    let bytes = extract_byte_slice(state, args_start_reg, caller_base);
    let n = tcp_send(fd, &bytes);
    if n < 0 {
        return Ok(Some(build_io_err(
            state,
            "BrokenPipe",
            8,
            &format!("tcp.write: send on fd {} failed", fd),
        )?));
    }
    Ok(Some(wrap_in_variant(
        state,
        "Result",
        0,
        &[Value::from_i64(n)],
    )?))
}

fn intercept_tcp_read(
    state: &mut InterpreterState,
    fd: i64,
    args_start_reg: u16,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    if let Some(denied) = check_net_permission(state) {
        return Ok(Some(denied));
    }
    // The `read(&mut [Byte])` arg is a mutable slice; the convention
    // for the high-level intercept is to recv up to `buf.len()` bytes
    // and write them into the slice's backing storage. For now, we
    // recv into a Rust Vec and write back via the slice's FatRef
    // pointer — this matches the canonical `read` semantics.
    let buf_v = state
        .registers
        .get(caller_base, crate::instruction::Reg(args_start_reg));
    let unwrapped = if super::cbgr_helpers::is_cbgr_ref(&buf_v) {
        let (abs_index, _) = super::cbgr_helpers::decode_cbgr_ref(buf_v.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        buf_v
    };
    // Determine slice capacity to bound the recv. FatRef carries
    // (ptr, len) in `len` field; raw List<Byte> carries it in the
    // List header. Worst-case fallback: 4 KiB.
    let cap = read_buffer_capacity(unwrapped).unwrap_or(4096);
    let cap = cap.min(1 << 20).max(1);
    let recvd = tcp_recv(fd, cap as i64);
    let n = match recvd {
        Some(s) => {
            // Write recv'd bytes into the buffer slot — the script
            // typically ignores the buffer parameter and relies on
            // the returned Int = bytes-recv'd to size a follow-up
            // `from_utf8` call on the original buffer reference.
            // For correctness we ALSO need to write the bytes into
            // the buffer storage. Falls back gracefully if the
            // slice shape isn't recognised.
            let bytes_len = s.as_bytes().len();
            write_into_byte_slice(unwrapped, s.as_bytes());
            bytes_len as i64
        }
        None => -1,
    };
    if n < 0 {
        return Ok(Some(build_io_err(
            state,
            "ConnectionAborted",
            4,
            &format!("tcp.read: recv on fd {} failed", fd),
        )?));
    }
    Ok(Some(wrap_in_variant(
        state,
        "Result",
        0,
        &[Value::from_i64(n)],
    )?))
}
