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
//!   * `__tcp_listen_raw(port: Int) -> Int`             — bind 0.0.0.0:port, listen.
//!   * `__tcp_accept_raw(fd: Int) -> Int`               — blocking accept.
//!   * `__tcp_connect_raw(host: Text, port: Int) -> Int` — TCP connect.
//!   * `__tcp_send_raw(fd: Int, data: Text) -> Int`     — send-all, returns bytes or -1.
//!   * `__tcp_recv_raw(fd: Int, max_len: Int) -> Text`  — single read.
//!   * `__tcp_close_raw(fd: Int) -> Int`                — drop registration.
//!   * `__udp_bind_raw(port: Int) -> Int`               — bind 0.0.0.0:port.
//!   * `__udp_send_raw(fd, data, host, port) -> Int`    — send_to.
//!   * `__udp_recv_raw(fd: Int, max_len: Int) -> Text`  — recv (peer ignored).
//!   * `__udp_close_raw(fd: Int) -> Int`
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
///   accept (NOT a kernel fd; see module-level docs).
/// * `-errno` on bind/listen failure — caller maps to IoErrorKind via
///   `core/io/protocols.vr::from_raw_os_error`.
/// * `-EINVAL` (`-22` Linux / `-22` macOS) for argument-validation
///   failures (bad host, port out of range, negative backlog).
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

    register(NetResource::Listener(listener))
}

/// Returns the OS-assigned local port of a registered TCP listener (or
/// connected stream). Used to recover the kernel-chosen port after
/// `tcp_listen_v2(_, 0, _, _)`. Returns `-1` for unknown fd or if the
/// underlying `getsockname(2)` fails.
pub fn tcp_local_port(fd: i64) -> i64 {
    REGISTRY.with(|r| {
        let map = r.borrow();
        match map.get(&fd) {
            Some(NetResource::Listener(l)) => {
                l.local_addr().map(|a| a.port() as i64).unwrap_or(-1)
            }
            Some(NetResource::Stream(s)) => {
                s.local_addr().map(|a| a.port() as i64).unwrap_or(-1)
            }
            Some(NetResource::Udp(s)) => {
                s.local_addr().map(|a| a.port() as i64).unwrap_or(-1)
            }
            None => -1,
        }
    })
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
    let listener = match listener {
        Some(l) => l,
        None => return -1,
    };
    let result = listener.accept();
    // Re-register the listener so a subsequent accept() call sees it.
    REGISTRY.with(|r| {
        r.borrow_mut().insert(listen_fd, NetResource::Listener(listener));
    });
    match result {
        Ok((stream, _peer)) => register(NetResource::Stream(stream)),
        Err(_) => -1,
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
    // Write under the borrow because we just need a &mut TcpStream.
    REGISTRY.with(|r| {
        let mut map = r.borrow_mut();
        match map.get_mut(&fd) {
            Some(NetResource::Stream(s)) => {
                match s.write_all(data) {
                    Ok(()) => data.len() as i64,
                    Err(_) => -1,
                }
            }
            _ => -1,
        }
    })
}

pub fn tcp_recv(fd: i64, max_len: i64) -> Option<String> {
    if max_len <= 0 {
        return Some(String::new());
    }
    let cap = max_len.min(1 << 20) as usize; // hard-cap 1 MiB / call.
    let mut buf = vec![0_u8; cap];
    let n = REGISTRY.with(|r| {
        let mut map = r.borrow_mut();
        match map.get_mut(&fd) {
            Some(NetResource::Stream(s)) => s.read(&mut buf).ok(),
            _ => None,
        }
    })?;
    buf.truncate(n);
    Some(String::from_utf8_lossy(&buf).into_owned())
}

pub fn tcp_close(fd: i64) -> i64 {
    REGISTRY.with(|r| {
        let mut map = r.borrow_mut();
        match map.remove(&fd) {
            Some(_) => 0,
            None => -1,
        }
    })
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
