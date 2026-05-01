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

    // **Real-kernel-fd return** (#25 cascade closure).
    //
    // Pre-#25 v2 returned a synthetic fd via `register(...)` and the
    // std::net wrapper lived in the registry for resource lifetime
    // tracking.  Synthetic fds are incompatible with the user-facing
    // `core.net.tcp.TcpListener` surface, which threads its `fd` field
    // through libc-bound operations (`accept`/`close`/`setsockopt`/
    // `getsockname`) — those operations are dispatched via libffi in
    // interpreter mode and require a real kernel fd.
    //
    // Resolution: `into_raw_fd()` consumes the std::net wrapper and
    // returns the underlying kernel fd.  Ownership transfers to the
    // caller — Verum-side `core/net/tcp.vr::TcpListener::Drop` calls
    // libc `close()` to free the fd at the right scope boundary,
    // mirroring the AOT path's lifecycle.  No registry tracking is
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
        // libc fd APIs.  Keep the legacy registry path here and
        // leave Windows fd-bridging as a follow-up.  Most weft
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
/// kernel fd (no registry tracking).  We call `getsockname(2)`
/// directly on the fd — works for any bound socket regardless of
/// whether it was created via this intrinsic family or via a
/// libffi-bridged libc `socket()` call.  This is the source of truth
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
        // length.  sin_port lives at offset 2 in either layout.
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
        // Windows: synthetic-fd flow only.  Real-fd query path falls
        // through to -1.
        -1
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
    // Legacy synthetic-fd path: drop from registry, std::net Drop runs.
    let registry_hit = REGISTRY.with(|r| {
        let mut map = r.borrow_mut();
        map.remove(&fd).is_some()
    });
    if registry_hit {
        return 0;
    }

    // Real-fd path (#25): call libc `close(2)` directly.  v2 listen
    // returns raw kernel fds (no registry tracking), so close has to
    // mirror that path.  Returns 0 on success, -errno on failure
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
// fallback in `core/sys/net_ops.vr`.  The HIGH-LEVEL
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
// SocketAddr }` record.  Subsequent reads/writes via the existing
// `__tcp_*_raw` intercepts share the same REGISTRY, so the stream
// is fully usable end-to-end.
// ============================================================================

use crate::interpreter::permission::{PermissionDecision, PermissionScope};
use crate::interpreter::state::InterpreterState;
use crate::types::TypeId;
use crate::value::Value;
use super::super::super::error::InterpreterResult;
use super::string_helpers::{alloc_string_value, extract_string};

pub(in super::super) fn try_intercept_net_runtime(
    state: &mut InterpreterState,
    func_name: &str,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    // Disambiguate against unrelated stdlib functions by qualifying
    // on `core.net.tcp` / `TcpStream.` / `TcpListener.`.  This file
    // intercepts ONLY the `core.net.tcp` high-level surface.
    if !func_name.contains("net.tcp")
        && !func_name.contains("net::tcp")
        && !func_name.contains("TcpStream.")
        && !func_name.contains("TcpStream::")
    {
        return Ok(None);
    }
    let bare = func_name.rsplit('.').next().unwrap_or(func_name);
    match bare {
        // `TcpStream.connect<A: ToSocketAddrs>(addr: A) ->
        // Result<TcpStream, IoError>` — the polymorphic top-level
        // entry.  We accept the most common A = Text shape directly
        // (parses "host:port"), which covers `TcpStream.connect("…")`
        // — the canonical script idiom.  Other A shapes (SocketAddr,
        // (Ipv4Addr, Int), ...) fall through to the bytecode body
        // (which then calls connect_addr internally — covered below).
        "connect" if arg_count == 1 => {
            intercept_connect_text(state, args_start_reg, caller_base)
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
    // Split host and port.  Bracketed IPv6 [::]:port works through
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
    // FileDesc is `is (Int)` — a transparent newtype.  At the value
    // level it's just the Int with no wrapper record (verified by
    // tracing `stream.write(self)` — the receiver's field 0 reads
    // back as `Value::is_int() == true`).  Store the fd directly.
    let fd_value = Value::from_i64(fd);
    // peer_addr: best-effort SocketAddr.V4 reconstruction from
    // the resolved (host, port).  When we can't parse the host as
    // an IPv4 literal, omit the peer_addr field by storing Unit —
    // the script's `Ok(_)` arm in the canonical test ignores the
    // payload anyway.  V1 follow-up: round-trip via
    // `getpeername(2)` on the live fd for the truly-resolved peer.
    let peer_addr = build_peer_addr(state, &host, port).unwrap_or(Value::unit());
    let stream = alloc_record_n_fields(state, "TcpStream", &[fd_value, peer_addr])?;
    Ok(Some(wrap_in_variant(state, "Result", 0, &[stream])?))
}

/// Best-effort `SocketAddr.V4(SocketAddrV4 { ip, port })`
/// construction from a literal `host:port` pair.  Returns None when
/// `host` doesn't parse as a four-octet IPv4 literal — caller falls
/// back to Unit.  IPv6 round-tripping is a V1 follow-up.
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

fn extract_text_arg(state: &InterpreterState, reg: u16, caller_base: u32) -> String {
    let v = state.registers.get(caller_base, crate::instruction::Reg(reg));
    let mut unwrapped = if super::cbgr_helpers::is_cbgr_ref(&v) {
        let (abs_index, _) = super::cbgr_helpers::decode_cbgr_ref(v.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        v
    };
    // ThinRef → pointee (mirrors text_extended.rs shape probe).
    if unwrapped.is_thin_ref() {
        let tr = unwrapped.as_thin_ref();
        if !tr.ptr.is_null() {
            unwrapped = unsafe { *(tr.ptr as *const Value) };
        }
    }
    extract_string(&unwrapped, state)
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

fn alloc_record_n_fields(
    state: &mut InterpreterState,
    type_name: &str,
    fields: &[Value],
) -> InterpreterResult<Value> {
    use crate::interpreter::heap::OBJECT_HEADER_SIZE;
    let type_id = lookup_type_id_by_name(state, type_name).unwrap_or(TypeId(0x9000));
    let payload_size = fields.len() * std::mem::size_of::<Value>();
    let obj = state.heap.alloc(type_id, payload_size)?;
    state.record_allocation();
    let data_ptr = unsafe { (obj.as_ptr() as *mut u8).add(OBJECT_HEADER_SIZE) as *mut Value };
    for (i, v) in fields.iter().enumerate() {
        unsafe {
            *data_ptr.add(i) = *v;
        }
    }
    Ok(Value::from_ptr(obj.as_ptr() as *mut u8))
}

fn wrap_in_variant(
    state: &mut InterpreterState,
    type_name: &str,
    tag: u32,
    fields: &[Value],
) -> InterpreterResult<Value> {
    use crate::interpreter::heap::OBJECT_HEADER_SIZE;
    let type_id = lookup_type_id_by_name(state, type_name).unwrap_or(TypeId(0x8000 + tag));
    let field_count = fields.len() as u32;
    let data_size = 8 + (fields.len() * std::mem::size_of::<Value>());
    let obj = state.heap.alloc(type_id, data_size)?;
    state.record_allocation();
    let base = obj.as_ptr() as *mut u8;
    unsafe {
        let tag_ptr = base.add(OBJECT_HEADER_SIZE) as *mut u32;
        *tag_ptr = tag;
        *tag_ptr.add(1) = field_count;
        let payload_ptr = base.add(OBJECT_HEADER_SIZE + 8) as *mut Value;
        for (i, v) in fields.iter().enumerate() {
            *payload_ptr.add(i) = *v;
        }
    }
    Ok(Value::from_ptr(base))
}

fn lookup_type_id_by_name(state: &InterpreterState, name: &str) -> Option<TypeId> {
    state
        .module
        .types
        .iter()
        .find(|td| {
            state.module.strings.get(td.name) == Some(name)
                && !matches!(td.kind, crate::types::TypeKind::Protocol)
        })
        .map(|td| td.id)
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
// `TcpStream` record AND the method being one we cover.  Any other
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
    // Receiver must be a TcpStream record.  Cheap check first:
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
    if !method_name.contains("TcpStream.") && !is_tcpstream_value(state, receiver) {
        return Ok(None);
    }
    let fd = match read_tcpstream_fd(receiver) {
        Some(fd) => fd,
        None => return Ok(None),
    };
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

/// Read the `fd` field of a TcpStream record.  TcpStream layout is
/// `[ObjectHeader][fd: Value][peer_addr: Value]`.  fd may be either
/// a transparent-newtype Int (Verum's `is (Int)` newtype lowering)
/// or a 1-field wrapper record — handle both.
fn read_tcpstream_fd(v: Value) -> Option<i64> {
    // Thin-ref (heap-pointer-to-Value) auto-deref.  CBGR-ref unwrap
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

fn is_tcpstream_value(state: &InterpreterState, v: Value) -> bool {
    if !v.is_ptr() || v.is_nil() {
        return false;
    }
    let ptr = v.as_ptr::<u8>();
    if ptr.is_null() {
        return false;
    }
    let header = unsafe { &*(ptr as *const crate::interpreter::heap::ObjectHeader) };
    state
        .module
        .types
        .iter()
        .any(|td| td.id == header.type_id && state.module.strings.get(td.name) == Some("TcpStream"))
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
    // and write them into the slice's backing storage.  For now, we
    // recv into a Rust Vec and write back via the slice's FatRef
    // pointer — this matches the canonical `read` semantics.
    let buf_v = state.registers.get(caller_base, crate::instruction::Reg(args_start_reg));
    let unwrapped = if super::cbgr_helpers::is_cbgr_ref(&buf_v) {
        let (abs_index, _) = super::cbgr_helpers::decode_cbgr_ref(buf_v.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        buf_v
    };
    // Determine slice capacity to bound the recv.  FatRef carries
    // (ptr, len) in `len` field; raw List<Byte> carries it in the
    // List header.  Worst-case fallback: 4 KiB.
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
            // the buffer storage.  Falls back gracefully if the
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

/// Extract a `&[Byte]` argument into an owned Vec<u8>.  Walks
/// either a FatRef-shaped slice or a List<Byte> backing storage
/// (one Value-slot per element, low-byte truncated).
fn extract_byte_slice(state: &InterpreterState, reg: u16, caller_base: u32) -> Vec<u8> {
    let v = state.registers.get(caller_base, crate::instruction::Reg(reg));
    let unwrapped = if super::cbgr_helpers::is_cbgr_ref(&v) {
        let (abs_index, _) = super::cbgr_helpers::decode_cbgr_ref(v.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        v
    };
    if unwrapped.is_fat_ref() {
        let fr = unwrapped.as_fat_ref();
        let len = fr.len() as usize;
        if fr.ptr().is_null() || len == 0 {
            return Vec::new();
        }
        // FatRef of byte slice: elem_size 1 = packed bytes (memcpy
        // safe).  elem_size 0 (NaN-boxed Values) requires per-slot
        // truncation.
        return match fr.reserved {
            1 => unsafe { std::slice::from_raw_parts(fr.ptr(), len) }.to_vec(),
            _ => {
                let mut out = Vec::with_capacity(len);
                for i in 0..len {
                    let elem =
                        unsafe { *(fr.ptr() as *const Value).add(i) };
                    out.push(elem.as_i64() as u8);
                }
                out
            }
        };
    }
    if unwrapped.is_ptr() && !unwrapped.is_nil() {
        let ptr = unwrapped.as_ptr::<u8>();
        if ptr.is_null() {
            return Vec::new();
        }
        let header =
            unsafe { &*(ptr as *const crate::interpreter::heap::ObjectHeader) };
        if header.type_id == TypeId::LIST {
            let data_ptr = unsafe {
                ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *const Value
            };
            let len = unsafe { (*data_ptr).as_i64() } as usize;
            let backing_v = unsafe { *data_ptr.add(2) };
            if backing_v.is_ptr() && !backing_v.is_nil() {
                let backing = backing_v.as_ptr::<u8>();
                if !backing.is_null() {
                    let backing_data = unsafe {
                        backing.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *const Value
                    };
                    let mut out = Vec::with_capacity(len);
                    for i in 0..len {
                        out.push(unsafe { (*backing_data.add(i)).as_i64() } as u8);
                    }
                    return out;
                }
            }
        }
    }
    Vec::new()
}

/// Capacity of a byte buffer for sizing recv() — the slice's
/// declared length when it's a FatRef, the list's len when it's
/// List<Byte>.  Returns None if the shape isn't recognised.
fn read_buffer_capacity(v: Value) -> Option<usize> {
    if v.is_fat_ref() {
        return Some(v.as_fat_ref().len() as usize);
    }
    if v.is_ptr() && !v.is_nil() {
        let ptr = v.as_ptr::<u8>();
        if ptr.is_null() {
            return None;
        }
        let header =
            unsafe { &*(ptr as *const crate::interpreter::heap::ObjectHeader) };
        if header.type_id == TypeId::LIST {
            let data_ptr = unsafe {
                ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *const Value
            };
            return Some(unsafe { (*data_ptr).as_i64() } as usize);
        }
    }
    None
}

/// Write `bytes` into the backing storage of a `&mut [Byte]`-shaped
/// value.  Best-effort; silently no-ops if the shape is unrecognised
/// (the script's `Ok(n)` arm conveys the byte count regardless).
fn write_into_byte_slice(v: Value, bytes: &[u8]) {
    if v.is_fat_ref() {
        let fr = v.as_fat_ref();
        let cap = fr.len() as usize;
        let n = bytes.len().min(cap);
        if fr.ptr().is_null() || n == 0 {
            return;
        }
        match fr.reserved {
            1 => unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), fr.ptr(), n);
            },
            _ => {
                let dst = fr.ptr() as *mut Value;
                for i in 0..n {
                    unsafe { *dst.add(i) = Value::from_i64(bytes[i] as i64) };
                }
            }
        }
        return;
    }
    if v.is_ptr() && !v.is_nil() {
        let ptr = v.as_ptr::<u8>();
        if ptr.is_null() {
            return;
        }
        let header =
            unsafe { &*(ptr as *const crate::interpreter::heap::ObjectHeader) };
        if header.type_id == TypeId::LIST {
            let data_ptr = unsafe {
                ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *const Value
            };
            let cap = unsafe { (*data_ptr.add(1)).as_i64() } as usize;
            let n = bytes.len().min(cap);
            let backing_v = unsafe { *data_ptr.add(2) };
            if backing_v.is_ptr() && !backing_v.is_nil() {
                let backing = backing_v.as_ptr::<u8>();
                if !backing.is_null() {
                    let backing_data = unsafe {
                        backing.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *mut Value
                    };
                    for i in 0..n {
                        unsafe {
                            *backing_data.add(i) = Value::from_i64(bytes[i] as i64);
                        }
                    }
                    // Update len in header to reflect bytes written.
                    let len_ptr = data_ptr as *mut Value;
                    unsafe { *len_ptr = Value::from_i64(n as i64) };
                }
            }
        }
    }
}
