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

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};

/// The kinds of network resources we track per fd.
enum NetResource {
    Listener(TcpListener),
    Stream(TcpStream),
    Udp(UdpSocket),
}

thread_local! {
    static REGISTRY: RefCell<HashMap<i64, NetResource>> = RefCell::new(HashMap::new());
    static NEXT_FD: Cell<i64> = const { Cell::new(1) };
}

fn alloc_fd() -> i64 {
    NEXT_FD.with(|c| {
        let v = c.get();
        c.set(v.wrapping_add(1));
        v
    })
}

fn register(res: NetResource) -> i64 {
    let fd = alloc_fd();
    REGISTRY.with(|r| r.borrow_mut().insert(fd, res));
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
    let from_registry = REGISTRY.with(|r| {
        let map = r.borrow();
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
    });
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
    let from_registry = REGISTRY.with(|r| {
        let map = r.borrow();
        match map.get(&fd) {
            Some(NetResource::Stream(s)) => s.peer_addr().ok(),
            _ => None,
        }
    });
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
                sa[8], sa[9], sa[10], sa[11], sa[12], sa[13], sa[14], sa[15],
                sa[16], sa[17], sa[18], sa[19], sa[20], sa[21], sa[22], sa[23],
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
    let listener: Option<TcpListener> = REGISTRY.with(|r| {
        let mut map = r.borrow_mut();
        match map.remove(&listen_fd) {
            Some(NetResource::Listener(l)) => Some(l),
            other => {
                if let Some(o) = other {
                    map.insert(listen_fd, o);
                }
                None
            }
        }
    });
    if let Some(listener) = listener {
        // Synthetic-fd path (legacy `__tcp_listen_raw`).
        let result = listener.accept();
        REGISTRY.with(|r| {
            r.borrow_mut().insert(listen_fd, NetResource::Listener(listener));
        });
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
    // First try the synthetic-fd registry path (legacy `__tcp_*_raw`).
    let registry_result: Option<i64> = REGISTRY.with(|r| {
        let mut map = r.borrow_mut();
        match map.get_mut(&fd) {
            Some(NetResource::Stream(s)) => {
                Some(match s.write_all(data) {
                    Ok(()) => data.len() as i64,
                    Err(_) => -1,
                })
            }
            _ => None,
        }
    });
    if let Some(rc) = registry_result {
        return rc;
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
    // Synthetic-fd registry path first.
    let registry_result: Option<Option<usize>> = REGISTRY.with(|r| {
        let mut map = r.borrow_mut();
        match map.get_mut(&fd) {
            Some(NetResource::Stream(s)) => Some(s.read(&mut buf).ok()),
            _ => None,
        }
    });
    if let Some(read_result) = registry_result {
        let n = read_result?;
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
    let registry_hit = REGISTRY.with(|r| {
        let mut map = r.borrow_mut();
        map.remove(&fd).is_some()
    });
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
    REGISTRY.with(|r| {
        let map = r.borrow();
        match map.get(&fd) {
            Some(NetResource::Udp(s)) => {
                match s.send_to(data, (host, port as u16)) {
                    Ok(n) => n as i64,
                    Err(_) => -1,
                }
            }
            _ => -1,
        }
    })
}

pub fn udp_recv(fd: i64, max_len: i64) -> Option<String> {
    if max_len <= 0 {
        return Some(String::new());
    }
    let cap = max_len.min(1 << 20) as usize;
    let mut buf = vec![0_u8; cap];
    let n = REGISTRY.with(|r| {
        let map = r.borrow();
        match map.get(&fd) {
            Some(NetResource::Udp(s)) => s.recv(&mut buf).ok(),
            _ => None,
        }
    })?;
    buf.truncate(n);
    Some(String::from_utf8_lossy(&buf).into_owned())
}

pub fn udp_close(fd: i64) -> i64 {
    tcp_close(fd) // same semantics — drop registration.
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn tcp_listen_accept_send_recv_round_trip() {
        // Server side
        let listen_fd = tcp_listen(0);
        assert!(listen_fd > 0);
        // Need the actual port to connect — read off the registered listener.
        let port = REGISTRY.with(|r| {
            let map = r.borrow();
            match map.get(&listen_fd) {
                Some(NetResource::Listener(l)) => l.local_addr().unwrap().port(),
                _ => panic!("listener missing"),
            }
        });
        // Spawn a client
        let client = thread::spawn(move || {
            // Tiny sleep so accept() is reached first deterministically.
            thread::sleep(Duration::from_millis(20));
            let cfd = tcp_connect("127.0.0.1", port as i64);
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
        assert_eq!(tcp_listen_v2("0.0.0.0", 70_000, 128, 0), -(libc::EINVAL as i64));
    }

    #[test]
    fn tcp_listen_v2_invalid_backlog_returns_einval() {
        assert_eq!(tcp_listen_v2("0.0.0.0", 0, -1, 0), -(libc::EINVAL as i64));
        assert_eq!(tcp_listen_v2("0.0.0.0", 0, 70_000, 0), -(libc::EINVAL as i64));
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

use crate::interpreter::permission::{PermissionDecision, PermissionScope};
use crate::interpreter::state::InterpreterState;
use crate::value::Value;
use super::super::super::error::InterpreterResult;
use super::heap_helpers::{
    alloc_record_n_fields, extract_byte_slice, extract_text_arg, is_record_typed_as,
    read_buffer_capacity, wrap_in_variant, write_into_byte_slice,
};
use super::string_helpers::alloc_string_value;

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
    if let Some(denied) = check_net_permission(state) {
        return Ok(Some(denied));
    }
    let addr_text = extract_text_arg(state, args_start_reg, caller_base);
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
            Ok(n) => (h.trim_matches(|c| c == '[' || c == ']').to_string(), n as i64),
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
    if let Some(denied) = check_net_permission(state) {
        return Ok(Some(denied));
    }
    let addr_text = extract_text_arg(state, args_start_reg, caller_base);
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
            &format!("listener.bind: tcp_listen_v2({}:{}) failed errno={}", host, port, -fd),
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
        build_peer_addr_v6(state, &host, bound_port).unwrap_or_else(|| {
            build_peer_addr(state, &host, bound_port).unwrap_or(Value::unit())
        })
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
    if let Some(denied) = check_net_permission(state) {
        return Ok(Some(denied));
    }
    let addr_text = extract_text_arg(state, args_start_reg, caller_base);
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
    let socket = alloc_record_n_fields(
        state,
        "UdpSocket",
        &[fd_value, local_addr, peer_addr_none],
    )?;
    Ok(Some(wrap_in_variant(state, "Result", 0, &[socket])?))
}

/// Best-effort `SocketAddr.V4(SocketAddrV4 { ip, port })`
/// construction from a literal `host:port` pair. Returns None when
/// `host` doesn't parse as a four-octet IPv4 literal — caller falls
/// back to Unit. IPv6 round-tripping is a V1 follow-up.
fn build_peer_addr(
    state: &mut InterpreterState,
    host: &str,
    port: i64,
) -> Option<Value> {
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

/// IPv6 sibling to `build_peer_addr`. Constructs
/// `SocketAddr.V6(SocketAddrV6 { ip: Ipv6Addr { (s0..s7) }, port,
/// flowinfo: 0, scope_id: 0 })` from an IPv6 literal in any
/// canonical form (`::1`, `2001:db8::1`, `[::]`). Returns None
/// when the literal doesn't parse — caller falls back to Unit.
/// Used by VBC-NET-4 for IPv6 peer_addr round-trip.
fn build_peer_addr_v6(
    state: &mut InterpreterState,
    host: &str,
    port: i64,
) -> Option<Value> {
    let host = host.trim_matches(|c| c == '[' || c == ']');
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

fn check_net_permission(state: &mut InterpreterState) -> Option<Value> {
    if state.check_permission(PermissionScope::Network, 0) == PermissionDecision::Deny {
        return build_io_err(
            state,
            "PermissionDenied",
            1,
            "permission denied: network access requires `net`",
        )
        .ok();
    }
    None
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
    let stream_err =
        alloc_record_n_fields(state, "StreamError", &[kind_variant, msg_some])?;
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
    let is_udp = method_name.contains("UdpSocket.")
        || is_record_typed_as(state, receiver, "UdpSocket");
    let is_listener = method_name.contains("TcpListener.")
        || is_record_typed_as(state, receiver, "TcpListener");
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
            "accept" if arg_count == 0 => {
                intercept_listener_accept(state, fd)
            }
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
    Ok(Some(wrap_in_variant(state, "Result", 0, &[Value::from_i64(n)])?))
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
    let s = udp_recv(fd, cap as i64);
    let n = match s {
        Some(s) => {
            let bytes = s.as_bytes();
            write_into_byte_slice(unwrapped, bytes);
            bytes.len() as i64
        }
        None => -1,
    };
    if n < 0 {
        return Ok(Some(build_io_err(
            state,
            "ConnectionAborted",
            4,
            &format!("recv_from: udp_recv on fd {} failed", fd),
        )?));
    }
    // Synthesise a placeholder peer SocketAddr.V4(0.0.0.0:0).
    let peer_addr = build_peer_addr(state, "0.0.0.0", 0).unwrap_or(Value::unit());
    let pair = alloc_record_n_fields(state, "Tuple", &[Value::from_i64(n), peer_addr])?;
    Ok(Some(wrap_in_variant(state, "Result", 0, &[pair])?))
}

/// Decode a Verum SocketAddr (sum of V4 / V6) — variant payload at
/// `[ObjectHeader][tag:u32][n_fields:u32][payload: Value]`. The
/// payload pointer leads to a 2-field SocketAddrV4 record (ip,
/// port) where the ip is a 4-byte tuple inlined as 4 Value slots.
fn read_socket_addr_value(v: Value) -> Option<(String, i64)> {
    if !v.is_ptr() || v.is_nil() {
        return None;
    }
    let ptr = v.as_ptr::<u8>();
    if ptr.is_null() {
        return None;
    }
    let tag = unsafe { *(ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *const u32) };
    let payload = unsafe {
        *(ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE + 8) as *const Value)
    };
    if tag == 0 {
        // V4: SocketAddrV4 { ip: Ipv4Addr { (a,b,c,d) }, port: Int }
        if !payload.is_ptr() || payload.is_nil() {
            return None;
        }
        let v4_ptr = payload.as_ptr::<u8>();
        if v4_ptr.is_null() {
            return None;
        }
        let v4_base = unsafe {
            v4_ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *const Value
        };
        let ip_v = unsafe { *v4_base };
        let port_v = unsafe { *v4_base.add(1) };
        if !ip_v.is_ptr() || ip_v.is_nil() {
            return None;
        }
        let ip_ptr = ip_v.as_ptr::<u8>();
        if ip_ptr.is_null() {
            return None;
        }
        let ip_base = unsafe {
            ip_ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *const Value
        };
        let a = unsafe { *ip_base }.as_i64() as u8;
        let b = unsafe { *ip_base.add(1) }.as_i64() as u8;
        let c = unsafe { *ip_base.add(2) }.as_i64() as u8;
        let d = unsafe { *ip_base.add(3) }.as_i64() as u8;
        Some((format!("{}.{}.{}.{}", a, b, c, d), port_v.as_i64()))
    } else {
        // V6 path — defer; the SocketAddrV6 layout is the same
        // 4-field record but ipv6 has 8 16-bit segments rather
        // than 4 bytes. Returning None falls through to
        // InvalidInput; covered by VBC-NET-4 follow-up.
        None
    }
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
    let fd_v = unsafe {
        *(ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *const Value)
    };
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
    Ok(Some(wrap_in_variant(state, "Result", 0, &[Value::from_i64(n)])?))
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
    let buf_v = state.registers.get(caller_base, crate::instruction::Reg(args_start_reg));
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
    Ok(Some(wrap_in_variant(state, "Result", 0, &[Value::from_i64(n)])?))
}

