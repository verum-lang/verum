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
}
