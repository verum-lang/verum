//! Tier-0 host-side `IoEngine` — backs the Verum-side abstraction
//! declared in `core/io/engine.vr`.
//!
//! VBC-IO-ENGINE-1 — closes the architectural gap between the
//! `AsyncIoReady` Future / `read_async` / `accept_async` surface in
//! `core/net/tcp.vr` and the kernel readiness primitives.  Pre-fix
//! the underlying intrinsics (`__io_engine_new_raw`,
//! `__io_submit_raw`, `__io_poll_raw`, …) were inert stubs returning
//! `1` (fake handle) / `0` / `-1`; the high-level Future was
//! theoretical.  Post-fix the intrinsics route through this module,
//! which mirrors the **Tier-1** `KqueueDriver` architecture from
//! `core/sys/darwin/io.vr` — each `IoEngine` handle owns its own
//! kqueue/epoll fd, registrations are level-triggered (re-fire on
//! every kevent until consumed), and `poll` returns ready fds via
//! a per-session ready queue accessible via `is_ready` queries.
//!
//! # Why a per-session kqueue (not the singleton reactor)?
//!
//! The singleton `crate::interpreter::reactor` is for ONE-SHOT
//! readiness waits (the `wait_readable(fd, timeout)` shape used by
//! `tcp_recv_timeout` etc.).  IoEngine is a different abstraction:
//! the user submits N fds, polls for ANY of them to be ready, then
//! queries `is_ready(fd, kind)` to dispatch.  The natural fit is one
//! kqueue/epoll fd per IoEngine session — exactly how the Verum-side
//! `KqueueDriver` is shaped.  Sharing the singleton reactor would
//! require routing reactor-fired events to the right session, which
//! adds complexity without benefit.
//!
//! # Architectural relationship to other modules
//!
//!  * `core/io/engine.vr::IoEngine` — Verum-side public surface;
//!    opaque `handle: Int` indexes into our session map.
//!  * `core/net/tcp.vr::AsyncIoReady` — Future that submit-then-
//!    polls-then-checks `is_ready` to drive its Poll<Output>.
//!  * `core/sys/io_engine.vr::IOEngine` (note capital E) — protocol
//!    declaration; `KqueueDriver` (Tier-1) implements it via
//!    Verum-side FFI to libSystem.  This module is the Tier-0
//!    counterpart on the Rust host side.
//!
//! # Public surface
//!
//!  * `engine_new(capacity) -> handle` (>0 on success, 0 on error).
//!  * `engine_destroy(handle) -> 0`.
//!  * `submit(handle, fd, flags) -> 0|-1` (flags: 1=Read, 2=Write,
//!    3=ReadWrite — matches `core/io/engine.vr::IoEvent.flags`).
//!  * `remove(handle, fd) -> 0|-1`.
//!  * `modify(handle, fd, flags) -> 0|-1`.
//!  * `poll(handle, max_events, timeout_ns) -> count` (≥0 = events
//!    delivered into session.ready; -1 = error).
//!  * `is_ready(handle, fd, flags) -> 1|0` — non-consuming query.
//!  * `take_ready(handle, fd, flags) -> 1|0` — consume the matched
//!    entry from the ready queue.
//!  * `async_accept(engine, listen_fd, timeout_ns) -> i64` — submit
//!    listen_fd for read-readiness, poll, accept once ready,
//!    register accepted stream in net_runtime REGISTRY, return the
//!    synthetic fd (or NET_STATUS_TIMEOUT / NET_STATUS_IO_ERROR
//!    on failure).

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

/// IoEvent flag bits (mirror of `core/io/engine.vr::IoEvent.flags`).
const FLAG_READ: i32 = 1;
const FLAG_WRITE: i32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IoFlags(i32);

impl IoFlags {
    fn from_raw(flags: i64) -> Self {
        IoFlags(flags as i32)
    }
    fn wants_read(&self) -> bool {
        (self.0 & FLAG_READ) != 0
    }
    fn wants_write(&self) -> bool {
        (self.0 & FLAG_WRITE) != 0
    }
}

/// A session — one kqueue/epoll fd + interest list + ready queue.
struct Session {
    /// kqueue fd (macOS/BSD) or epoll fd (Linux).
    backend_fd: i64,
    /// Registered fds → requested flags.
    interests: HashMap<i64, IoFlags>,
    /// Fired-but-not-consumed events.
    ready: Vec<(i64, IoFlags)>,
}

impl Session {
    #[cfg(target_os = "macos")]
    fn create() -> Option<Self> {
        let kq = unsafe { libc::kqueue() };
        if kq < 0 {
            return None;
        }
        unsafe {
            libc::fcntl(kq, libc::F_SETFD, libc::FD_CLOEXEC);
        }
        Some(Session {
            backend_fd: kq as i64,
            interests: HashMap::new(),
            ready: Vec::new(),
        })
    }

    #[cfg(target_os = "linux")]
    fn create() -> Option<Self> {
        let epfd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        if epfd < 0 {
            return None;
        }
        Some(Session {
            backend_fd: epfd as i64,
            interests: HashMap::new(),
            ready: Vec::new(),
        })
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn create() -> Option<Self> {
        None
    }

    #[cfg(target_os = "macos")]
    fn submit(&mut self, fd: i64, flags: IoFlags) -> i64 {
        // For ReadWrite, register both EVFILT_READ and EVFILT_WRITE.
        // Use EV_ADD | EV_ENABLE (level-triggered, re-fires until
        // consumed or fd becomes not-ready).  No EV_ONESHOT because
        // the IoEngine model is "drive until satisfied" — the
        // session's ready queue is the consumption point.
        if flags.wants_read() {
            let mut kev = libc::kevent {
                ident: fd as libc::uintptr_t,
                filter: libc::EVFILT_READ,
                flags: libc::EV_ADD | libc::EV_ENABLE,
                fflags: 0,
                data: 0,
                udata: std::ptr::null_mut(),
            };
            let rc = unsafe {
                libc::kevent(
                    self.backend_fd as libc::c_int,
                    &mut kev,
                    1,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null(),
                )
            };
            if rc < 0 {
                return -1;
            }
        }
        if flags.wants_write() {
            let mut kev = libc::kevent {
                ident: fd as libc::uintptr_t,
                filter: libc::EVFILT_WRITE,
                flags: libc::EV_ADD | libc::EV_ENABLE,
                fflags: 0,
                data: 0,
                udata: std::ptr::null_mut(),
            };
            let rc = unsafe {
                libc::kevent(
                    self.backend_fd as libc::c_int,
                    &mut kev,
                    1,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null(),
                )
            };
            if rc < 0 {
                return -1;
            }
        }
        self.interests.insert(fd, flags);
        0
    }

    #[cfg(target_os = "linux")]
    fn submit(&mut self, fd: i64, flags: IoFlags) -> i64 {
        let mut events: u32 = 0;
        if flags.wants_read() {
            events |= libc::EPOLLIN as u32;
        }
        if flags.wants_write() {
            events |= libc::EPOLLOUT as u32;
        }
        let mut ev = libc::epoll_event {
            events,
            u64: fd as u64,
        };
        // Try MOD first to handle re-submit; fall back to ADD.
        let rc_mod = unsafe {
            libc::epoll_ctl(
                self.backend_fd as libc::c_int,
                libc::EPOLL_CTL_MOD,
                fd as libc::c_int,
                &mut ev,
            )
        };
        if rc_mod != 0 {
            let rc_add = unsafe {
                libc::epoll_ctl(
                    self.backend_fd as libc::c_int,
                    libc::EPOLL_CTL_ADD,
                    fd as libc::c_int,
                    &mut ev,
                )
            };
            if rc_add < 0 {
                return -1;
            }
        }
        self.interests.insert(fd, flags);
        0
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn submit(&mut self, _fd: i64, _flags: IoFlags) -> i64 {
        -1
    }

    #[cfg(target_os = "macos")]
    fn remove(&mut self, fd: i64) -> i64 {
        let interests = self.interests.remove(&fd);
        if let Some(flags) = interests {
            if flags.wants_read() {
                let mut kev = libc::kevent {
                    ident: fd as libc::uintptr_t,
                    filter: libc::EVFILT_READ,
                    flags: libc::EV_DELETE,
                    fflags: 0,
                    data: 0,
                    udata: std::ptr::null_mut(),
                };
                let _ = unsafe {
                    libc::kevent(
                        self.backend_fd as libc::c_int,
                        &mut kev,
                        1,
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null(),
                    )
                };
            }
            if flags.wants_write() {
                let mut kev = libc::kevent {
                    ident: fd as libc::uintptr_t,
                    filter: libc::EVFILT_WRITE,
                    flags: libc::EV_DELETE,
                    fflags: 0,
                    data: 0,
                    udata: std::ptr::null_mut(),
                };
                let _ = unsafe {
                    libc::kevent(
                        self.backend_fd as libc::c_int,
                        &mut kev,
                        1,
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null(),
                    )
                };
            }
        }
        // Drop matching entries from ready queue.
        self.ready.retain(|(rfd, _)| *rfd != fd);
        0
    }

    #[cfg(target_os = "linux")]
    fn remove(&mut self, fd: i64) -> i64 {
        self.interests.remove(&fd);
        let _ = unsafe {
            libc::epoll_ctl(
                self.backend_fd as libc::c_int,
                libc::EPOLL_CTL_DEL,
                fd as libc::c_int,
                std::ptr::null_mut(),
            )
        };
        self.ready.retain(|(rfd, _)| *rfd != fd);
        0
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn remove(&mut self, _fd: i64) -> i64 {
        -1
    }

    fn modify(&mut self, fd: i64, flags: IoFlags) -> i64 {
        // Easiest correct path: remove + submit.
        let _ = self.remove(fd);
        self.submit(fd, flags)
    }

    /// Block up to `timeout_ns` for any registered fd to be ready.
    /// On return, `self.ready` contains the fired events (caller
    /// queries via `is_ready` / `take_ready`).
    #[cfg(target_os = "macos")]
    fn poll(&mut self, max_events: i64, timeout_ns: i64) -> i64 {
        let max = max_events.clamp(1, 256) as usize;
        let mut events: Vec<libc::kevent> =
            (0..max).map(|_| unsafe { std::mem::zeroed() }).collect();
        let timeout = if timeout_ns < 0 {
            std::ptr::null()
        } else {
            // Borrow the timespec by reference into the kevent call.
            let ts = libc::timespec {
                tv_sec: (timeout_ns / 1_000_000_000) as libc::time_t,
                tv_nsec: (timeout_ns % 1_000_000_000) as libc::c_long,
            };
            // SAFETY: ts lives until the kevent call returns; we
            // bind via Box to extend its lifetime through the FFI.
            let boxed: Box<libc::timespec> = Box::new(ts);
            let raw = Box::into_raw(boxed);
            // Schedule deallocation after the call below.
            let n = unsafe {
                libc::kevent(
                    self.backend_fd as libc::c_int,
                    std::ptr::null(),
                    0,
                    events.as_mut_ptr(),
                    max as libc::c_int,
                    raw,
                )
            };
            unsafe {
                let _ = Box::from_raw(raw);
            }
            return self.harvest_kevents(&events, n);
        };
        let n = unsafe {
            libc::kevent(
                self.backend_fd as libc::c_int,
                std::ptr::null(),
                0,
                events.as_mut_ptr(),
                max as libc::c_int,
                timeout,
            )
        };
        self.harvest_kevents(&events, n)
    }

    #[cfg(target_os = "macos")]
    fn harvest_kevents(&mut self, events: &[libc::kevent], n: libc::c_int) -> i64 {
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                return 0; // treat EINTR as "no events this round"
            }
            return -1;
        }
        for kev in &events[..n as usize] {
            let fd = kev.ident as i64;
            let flag = if kev.filter == libc::EVFILT_READ {
                FLAG_READ
            } else if kev.filter == libc::EVFILT_WRITE {
                FLAG_WRITE
            } else {
                continue;
            };
            self.ready.push((fd, IoFlags(flag)));
        }
        n as i64
    }

    #[cfg(target_os = "linux")]
    fn poll(&mut self, max_events: i64, timeout_ns: i64) -> i64 {
        let max = max_events.clamp(1, 256) as usize;
        let mut events: Vec<libc::epoll_event> =
            (0..max).map(|_| unsafe { std::mem::zeroed() }).collect();
        // epoll_wait takes timeout in ms.  Round up so we don't
        // return early on sub-ms timeouts.
        let timeout_ms: i32 = if timeout_ns < 0 {
            -1
        } else {
            ((timeout_ns + 999_999) / 1_000_000).clamp(0, i32::MAX as i64) as i32
        };
        let n = unsafe {
            libc::epoll_wait(
                self.backend_fd as libc::c_int,
                events.as_mut_ptr(),
                max as libc::c_int,
                timeout_ms,
            )
        };
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                return 0;
            }
            return -1;
        }
        for ev in &events[..n as usize] {
            let fd = ev.u64 as i64;
            // For each set bit in events, push a separate ready entry.
            if (ev.events & libc::EPOLLIN as u32) != 0 {
                self.ready.push((fd, IoFlags(FLAG_READ)));
            }
            if (ev.events & libc::EPOLLOUT as u32) != 0 {
                self.ready.push((fd, IoFlags(FLAG_WRITE)));
            }
            // POLLHUP / POLLERR — surface as the registered interest
            // so the caller's read/write surfaces the actual errno.
            if (ev.events & (libc::EPOLLHUP as u32 | libc::EPOLLERR as u32)) != 0 {
                if let Some(flags) = self.interests.get(&fd) {
                    if flags.wants_read() {
                        self.ready.push((fd, IoFlags(FLAG_READ)));
                    }
                    if flags.wants_write() {
                        self.ready.push((fd, IoFlags(FLAG_WRITE)));
                    }
                }
            }
        }
        n as i64
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn poll(&mut self, _max: i64, _timeout_ns: i64) -> i64 {
        -1
    }

    /// Non-consuming check: is `fd` ready for `flags`?
    fn is_ready(&self, fd: i64, flags: IoFlags) -> bool {
        self.ready.iter().any(|(rfd, rflags)| {
            *rfd == fd && (rflags.0 & flags.0) != 0
        })
    }

    /// Consuming check: returns true and removes the matching entry.
    fn take_ready(&mut self, fd: i64, flags: IoFlags) -> bool {
        if let Some(pos) = self
            .ready
            .iter()
            .position(|(rfd, rflags)| *rfd == fd && (rflags.0 & flags.0) != 0)
        {
            self.ready.remove(pos);
            true
        } else {
            false
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.backend_fd >= 0 {
            unsafe {
                libc::close(self.backend_fd as libc::c_int);
            }
        }
    }
}

// ============================================================================
// Session table
// ============================================================================

static SESSIONS: LazyLock<Mutex<HashMap<i64, Session>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_HANDLE: AtomicI64 = AtomicI64::new(1);

// ============================================================================
// Public surface (called from the intrinsic dispatcher)
// ============================================================================

pub fn engine_new(_capacity: i64) -> i64 {
    match Session::create() {
        Some(session) => {
            let h = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
            SESSIONS.lock().unwrap().insert(h, session);
            h
        }
        None => 0,
    }
}

pub fn engine_destroy(handle: i64) -> i64 {
    SESSIONS.lock().unwrap().remove(&handle);
    0
}

pub fn submit(handle: i64, fd: i64, flags: i64) -> i64 {
    let mut sessions = SESSIONS.lock().unwrap();
    match sessions.get_mut(&handle) {
        Some(s) => s.submit(fd, IoFlags::from_raw(flags)),
        None => -1,
    }
}

pub fn remove(handle: i64, fd: i64) -> i64 {
    let mut sessions = SESSIONS.lock().unwrap();
    match sessions.get_mut(&handle) {
        Some(s) => s.remove(fd),
        None => -1,
    }
}

pub fn modify(handle: i64, fd: i64, flags: i64) -> i64 {
    let mut sessions = SESSIONS.lock().unwrap();
    match sessions.get_mut(&handle) {
        Some(s) => s.modify(fd, IoFlags::from_raw(flags)),
        None => -1,
    }
}

/// Drop the SESSIONS lock around the (potentially blocking) poll
/// — concurrent submits / queries on OTHER handles must not stall.
/// The session itself is briefly removed from the map and re-
/// inserted after poll returns.  Concurrent polls on the SAME
/// handle return -1 (the session is "in use").
pub fn poll(handle: i64, max_events: i64, timeout_ns: i64) -> i64 {
    let mut session = {
        let mut sessions = SESSIONS.lock().unwrap();
        match sessions.remove(&handle) {
            Some(s) => s,
            None => return -1,
        }
    };
    let result = session.poll(max_events, timeout_ns);
    SESSIONS.lock().unwrap().insert(handle, session);
    result
}

pub fn is_ready(handle: i64, fd: i64, flags: i64) -> i64 {
    let sessions = SESSIONS.lock().unwrap();
    match sessions.get(&handle) {
        Some(s) => {
            if s.is_ready(fd, IoFlags::from_raw(flags)) {
                1
            } else {
                0
            }
        }
        None => 0,
    }
}

pub fn take_ready(handle: i64, fd: i64, flags: i64) -> i64 {
    let mut sessions = SESSIONS.lock().unwrap();
    match sessions.get_mut(&handle) {
        Some(s) => {
            if s.take_ready(fd, IoFlags::from_raw(flags)) {
                1
            } else {
                0
            }
        }
        None => 0,
    }
}

// ============================================================================
// Socket option helpers (also intercept-backed)
// ============================================================================

/// Set / clear O_NONBLOCK on `fd`.  Returns 0 on success, -1 on failure.
#[cfg(unix)]
pub fn socket_set_nonblocking(fd: i64, on: bool) -> i64 {
    let flags = unsafe { libc::fcntl(fd as libc::c_int, libc::F_GETFL, 0) };
    if flags < 0 {
        return -1;
    }
    let new_flags = if on {
        flags | libc::O_NONBLOCK
    } else {
        flags & !libc::O_NONBLOCK
    };
    let rc = unsafe { libc::fcntl(fd as libc::c_int, libc::F_SETFL, new_flags) };
    if rc == 0 { 0 } else { -1 }
}
#[cfg(not(unix))]
pub fn socket_set_nonblocking(_fd: i64, _on: bool) -> i64 {
    -1
}

#[cfg(unix)]
fn set_int_sockopt(fd: i64, level: libc::c_int, name: libc::c_int, value: libc::c_int) -> i64 {
    let rc = unsafe {
        libc::setsockopt(
            fd as libc::c_int,
            level,
            name,
            &value as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if rc == 0 { 0 } else { -1 }
}
#[cfg(not(unix))]
fn set_int_sockopt(_fd: i64, _level: i32, _name: i32, _value: i32) -> i64 {
    -1
}

#[cfg(unix)]
pub fn socket_set_reuseaddr(fd: i64) -> i64 {
    set_int_sockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEADDR, 1)
}
#[cfg(not(unix))]
pub fn socket_set_reuseaddr(_fd: i64) -> i64 {
    -1
}

#[cfg(unix)]
pub fn socket_set_nodelay(fd: i64, on: bool) -> i64 {
    set_int_sockopt(
        fd,
        libc::IPPROTO_TCP,
        libc::TCP_NODELAY,
        if on { 1 } else { 0 },
    )
}
#[cfg(not(unix))]
pub fn socket_set_nodelay(_fd: i64, _on: bool) -> i64 {
    -1
}

#[cfg(unix)]
pub fn socket_set_keepalive(fd: i64, on: bool) -> i64 {
    set_int_sockopt(
        fd,
        libc::SOL_SOCKET,
        libc::SO_KEEPALIVE,
        if on { 1 } else { 0 },
    )
}
#[cfg(not(unix))]
pub fn socket_set_keepalive(_fd: i64, _on: bool) -> i64 {
    -1
}

/// Read pending SO_ERROR — returns 0 if clean, the errno otherwise,
/// or -1 if getsockopt itself failed.
#[cfg(unix)]
pub fn socket_get_error(fd: i64) -> i64 {
    let mut so_err: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd as libc::c_int,
            libc::SOL_SOCKET,
            libc::SO_ERROR,
            &mut so_err as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if rc != 0 { -1 } else { so_err as i64 }
}
#[cfg(not(unix))]
pub fn socket_get_error(_fd: i64) -> i64 {
    -1
}

// ============================================================================
// async_accept — submit + poll + accept, bridging IoEngine to
// the existing net_runtime REGISTRY
// ============================================================================

/// Submit `listen_fd` for read-readiness on the IoEngine session,
/// poll up to `timeout_ns`, accept once ready, and register the
/// accepted stream in `net_runtime` REGISTRY.  Returns the synthetic
/// fd of the registered stream, or a negative status code:
///  * NET_STATUS_TIMEOUT  (-2): poll timed out before any event
///  * NET_STATUS_REACTOR_ERROR (-3): IoEngine handle invalid
///  * NET_STATUS_IO_ERROR (-4): accept syscall failed
#[cfg(unix)]
pub fn async_accept(engine: i64, listen_fd: i64, timeout_ns: i64) -> i64 {
    use super::dispatch_table::handlers::net_runtime;
    // Submit interest.  If submit fails, the engine handle is
    // bad — surface as REACTOR_ERROR.
    if submit(engine, listen_fd, FLAG_READ as i64) != 0 {
        return net_runtime::NET_STATUS_REACTOR_ERROR;
    }
    let n = poll(engine, 16, timeout_ns);
    if n < 0 {
        let _ = remove(engine, listen_fd);
        return net_runtime::NET_STATUS_REACTOR_ERROR;
    }
    if n == 0 {
        let _ = remove(engine, listen_fd);
        return net_runtime::NET_STATUS_TIMEOUT;
    }
    // Consume the read-ready entry so subsequent polls don't
    // double-count it.
    let _ = take_ready(engine, listen_fd, FLAG_READ as i64);
    let _ = remove(engine, listen_fd);
    // Now do the actual accept(2).  The fd is ready, so accept
    // returns immediately with a connection.  The caller's
    // `listen_fd` is a kernel fd (real-fd path of tcp_listen_v2).
    let mut peer: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut peer_len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    #[cfg(target_os = "linux")]
    let conn_fd = unsafe {
        libc::accept4(
            listen_fd as libc::c_int,
            &mut peer as *mut _ as *mut libc::sockaddr,
            &mut peer_len,
            libc::SOCK_CLOEXEC,
        )
    };
    #[cfg(not(target_os = "linux"))]
    let conn_fd = unsafe {
        libc::accept(
            listen_fd as libc::c_int,
            &mut peer as *mut _ as *mut libc::sockaddr,
            &mut peer_len,
        )
    };
    if conn_fd < 0 {
        return net_runtime::NET_STATUS_IO_ERROR;
    }
    // Register the accepted stream in the net_runtime REGISTRY so
    // downstream tcp_send / tcp_recv / tcp_close intrinsics can
    // address it via synthetic fd.
    use std::os::unix::io::FromRawFd;
    let stream = unsafe { std::net::TcpStream::from_raw_fd(conn_fd) };
    net_runtime::register_accepted_stream(stream)
}
#[cfg(not(unix))]
pub fn async_accept(_engine: i64, _listen_fd: i64, _timeout_ns: i64) -> i64 {
    -1
}

// ============================================================================
// VBC-IO-ENGINE-2 — zero-copy async_read / async_write
// ============================================================================
//
// Closes the deferred follow-up declared in
// `project_vbc_io_engine_2026-05-01.md`. Pre-fix the
// `__async_read_raw` / `__async_write_raw` intrinsics returned -1
// (stub). Now they wait for fd readiness through the IoEngine
// (submit FLAG_READ/WRITE → poll → take_ready → remove), then
// perform a single libc::read / libc::write into the caller-
// supplied raw buffer.
//
// **The buffer is a raw address.** The Verum-side surface
// (`core/io/engine.vr::IoEngine.{read, write}`) accepts
// `buf: Int` — the caller is in unsafe territory and is
// responsible for keeping the buffer alive across the call.
// We validate the obvious safety invariants (non-zero address,
// non-negative length) and treat the address as opaque past that.
//
// Returns:
//   * `>= 0`  — bytes read / written.
//   * `0`     — EOF (read) / nothing-to-send (write); also the
//               trivial-case `len == 0` short-circuit.
//   * `NET_STATUS_TIMEOUT` — poll timed out before the fd became
//                            ready.
//   * `NET_STATUS_REACTOR_ERROR` — submit/poll failed; the engine
//                                  handle was bad.
//   * `NET_STATUS_IO_ERROR` — the libc::read/write itself failed,
//                             OR a precondition (bad fd / bad
//                             buffer / bad len) tripped.
//
// Cross-platform: Unix-only via libc::read / libc::write. The
// non-unix stub returns -1 to match the existing `async_accept`
// pattern.
#[cfg(unix)]
pub fn async_read(engine: i64, fd: i64, buf_addr: i64, len: i64, timeout_ns: i64) -> i64 {
    use super::dispatch_table::handlers::net_runtime;
    if fd < 0 || buf_addr == 0 || len < 0 {
        return net_runtime::NET_STATUS_IO_ERROR;
    }
    if len == 0 {
        return 0;
    }
    if !await_ready(engine, fd, FLAG_READ as i64, timeout_ns) {
        // `await_ready` returns false on TIMEOUT *or* REACTOR_ERROR;
        // disambiguate via the engine-validity probe.
        return classify_wait_failure(engine);
    }
    let n = unsafe {
        libc::read(
            fd as libc::c_int,
            buf_addr as *mut libc::c_void,
            len as libc::size_t,
        )
    };
    if n < 0 {
        net_runtime::NET_STATUS_IO_ERROR
    } else {
        n as i64
    }
}
#[cfg(not(unix))]
pub fn async_read(_engine: i64, _fd: i64, _buf_addr: i64, _len: i64, _timeout_ns: i64) -> i64 {
    -1
}

#[cfg(unix)]
pub fn async_write(engine: i64, fd: i64, buf_addr: i64, len: i64, timeout_ns: i64) -> i64 {
    use super::dispatch_table::handlers::net_runtime;
    if fd < 0 || buf_addr == 0 || len < 0 {
        return net_runtime::NET_STATUS_IO_ERROR;
    }
    if len == 0 {
        return 0;
    }
    if !await_ready(engine, fd, FLAG_WRITE as i64, timeout_ns) {
        return classify_wait_failure(engine);
    }
    let n = unsafe {
        libc::write(
            fd as libc::c_int,
            buf_addr as *const libc::c_void,
            len as libc::size_t,
        )
    };
    if n < 0 {
        net_runtime::NET_STATUS_IO_ERROR
    } else {
        n as i64
    }
}
#[cfg(not(unix))]
pub fn async_write(_engine: i64, _fd: i64, _buf_addr: i64, _len: i64, _timeout_ns: i64) -> i64 {
    -1
}

/// Mirror of the submit / poll / take_ready / remove choreography
/// used by `async_accept`. Returns `true` iff the fd became
/// ready within the timeout. False outcomes (timeout vs. reactor
/// error) are disambiguated by the caller via
/// `classify_wait_failure`.
#[cfg(unix)]
fn await_ready(engine: i64, fd: i64, flags: i64, timeout_ns: i64) -> bool {
    if submit(engine, fd, flags) != 0 {
        // Mark ENGINE_SUBMIT_FAILED via a side channel so the caller
        // can disambiguate. We use a thread-local because the
        // existing surface returns plain bool; threading an enum
        // through every call site is over-engineering for one use.
        WAIT_LAST_FAILURE.with(|c| c.set(WaitFailure::ReactorError));
        return false;
    }
    let n = poll(engine, 16, timeout_ns);
    if n < 0 {
        let _ = remove(engine, fd);
        WAIT_LAST_FAILURE.with(|c| c.set(WaitFailure::ReactorError));
        return false;
    }
    if n == 0 {
        let _ = remove(engine, fd);
        WAIT_LAST_FAILURE.with(|c| c.set(WaitFailure::Timeout));
        return false;
    }
    let _ = take_ready(engine, fd, flags);
    let _ = remove(engine, fd);
    true
}

/// Translate the thread-local wait-failure marker into the
/// canonical NET_STATUS_*. Always called on the false path of
/// `await_ready`; defaults to `NET_STATUS_IO_ERROR` if the marker
/// was somehow not set (defensive — should never happen).
#[cfg(unix)]
fn classify_wait_failure(_engine: i64) -> i64 {
    use super::dispatch_table::handlers::net_runtime;
    WAIT_LAST_FAILURE.with(|c| match c.get() {
        WaitFailure::Timeout => net_runtime::NET_STATUS_TIMEOUT,
        WaitFailure::ReactorError => net_runtime::NET_STATUS_REACTOR_ERROR,
        WaitFailure::None => net_runtime::NET_STATUS_IO_ERROR,
    })
}

#[cfg(unix)]
#[derive(Clone, Copy)]
enum WaitFailure {
    None,
    Timeout,
    ReactorError,
}

#[cfg(unix)]
thread_local! {
    static WAIT_LAST_FAILURE: std::cell::Cell<WaitFailure> =
        const { std::cell::Cell::new(WaitFailure::None) };
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};
    use std::os::fd::AsRawFd;

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn engine_new_returns_positive_handle() {
        let h = engine_new(64);
        assert!(h > 0, "expected positive handle, got {h}");
        assert_eq!(engine_destroy(h), 0);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn poll_with_timeout_returns_zero_when_no_events() {
        let h = engine_new(64);
        // Register a listener that nobody connects to.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let fd = listener.as_raw_fd() as i64;
        assert_eq!(submit(h, fd, FLAG_READ as i64), 0);
        let start = std::time::Instant::now();
        let n = poll(h, 16, 50_000_000); // 50 ms
        let elapsed = start.elapsed();
        assert_eq!(n, 0, "expected no events, got {n}");
        assert!(elapsed >= Duration::from_millis(40) && elapsed < Duration::from_secs(1));
        assert_eq!(remove(h, fd), 0);
        assert_eq!(engine_destroy(h), 0);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn poll_signals_ready_on_pending_connection() {
        let h = engine_new(64);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let fd = listener.as_raw_fd() as i64;
        assert_eq!(submit(h, fd, FLAG_READ as i64), 0);
        // Connect from a bg thread to make the listener readable.
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            let _s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            std::thread::sleep(Duration::from_millis(50));
        });
        let n = poll(h, 16, 1_000_000_000); // 1 s
        assert!(n > 0, "expected event, got {n}");
        assert_eq!(is_ready(h, fd, FLAG_READ as i64), 1);
        assert_eq!(take_ready(h, fd, FLAG_READ as i64), 1);
        // After consume, is_ready returns 0.
        assert_eq!(is_ready(h, fd, FLAG_READ as i64), 0);
        handle.join().unwrap();
        assert_eq!(remove(h, fd), 0);
        assert_eq!(engine_destroy(h), 0);
    }

    /// VBC-IO-ENGINE-1 — `async_accept` end-to-end: bind, register
    /// listen_fd in a fresh IoEngine session, kick a connect from
    /// a bg thread, drive `async_accept` and verify the returned
    /// synthetic fd is valid + addressable through net_runtime.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn async_accept_round_trip_via_io_engine() {
        use super::super::dispatch_table::handlers::net_runtime::{
            tcp_close, tcp_connect, tcp_listen_v2, tcp_local_port,
            NET_STATUS_TIMEOUT,
        };
        let h = engine_new(8);
        assert!(h > 0);
        let listen_fd = tcp_listen_v2("127.0.0.1", 0, 8, 0);
        assert!(listen_fd > 0);
        let port = tcp_local_port(listen_fd);
        // Spawn a connector after a small delay so async_accept
        // has a chance to register interest before the SYN arrives.
        let bg = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            let cfd = tcp_connect("127.0.0.1", port);
            assert!(cfd > 0);
            std::thread::sleep(Duration::from_millis(50));
            tcp_close(cfd);
        });
        let server_fd = async_accept(h, listen_fd, 1_500_000_000); // 1.5s
        assert!(
            server_fd > 0 && server_fd != NET_STATUS_TIMEOUT,
            "async_accept returned {server_fd}"
        );
        bg.join().unwrap();
        assert_eq!(tcp_close(server_fd), 0);
        assert_eq!(tcp_close(listen_fd), 0);
        assert_eq!(engine_destroy(h), 0);
    }

    /// VBC-IO-ENGINE-1 — `async_accept` returns NET_STATUS_TIMEOUT
    /// when no client connects within the deadline.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn async_accept_times_out_when_no_client() {
        use super::super::dispatch_table::handlers::net_runtime::{
            tcp_close, tcp_listen_v2, NET_STATUS_TIMEOUT,
        };
        let h = engine_new(8);
        assert!(h > 0);
        let listen_fd = tcp_listen_v2("127.0.0.1", 0, 8, 0);
        assert!(listen_fd > 0);
        let start = std::time::Instant::now();
        let r = async_accept(h, listen_fd, 100_000_000); // 100 ms
        let elapsed = start.elapsed();
        assert_eq!(r, NET_STATUS_TIMEOUT);
        assert!(elapsed >= Duration::from_millis(80) && elapsed < Duration::from_secs(2));
        assert_eq!(tcp_close(listen_fd), 0);
        assert_eq!(engine_destroy(h), 0);
    }

    /// VBC-IO-ENGINE-2 — `async_read` end-to-end. Bind a
    /// listener, accept a connection, kick the peer to send 13
    /// bytes, then drive `async_read` against the server-side
    /// TcpStream's raw fd — verify the bytes land in the
    /// caller-supplied buffer.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn async_read_end_to_end_via_io_engine() {
        let h = engine_new(8);
        assert!(h > 0);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        // Background: connect + send 13 bytes.
        let bg = std::thread::spawn(move || {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            std::thread::sleep(Duration::from_millis(20));
            use std::io::Write;
            s.write_all(b"hello, world!").unwrap();
            std::thread::sleep(Duration::from_millis(80));
        });

        // Accept the connection synchronously to get the server-
        // side raw fd.
        let (stream, _) = listener.accept().unwrap();
        let server_fd = stream.as_raw_fd() as i64;
        // Drop the std wrapper but keep the raw fd alive — the
        // caller (us) now owns the fd lifecycle for the test.
        std::mem::forget(stream);

        let mut buf = [0u8; 32];
        let n = async_read(
            h,
            server_fd,
            buf.as_mut_ptr() as i64,
            buf.len() as i64,
            1_000_000_000, // 1s
        );
        assert_eq!(n, 13, "expected 13 bytes, got {n}");
        assert_eq!(&buf[..13], b"hello, world!");

        bg.join().unwrap();
        unsafe { libc::close(server_fd as libc::c_int) };
        assert_eq!(engine_destroy(h), 0);
    }

    /// VBC-IO-ENGINE-2 — `async_write` end-to-end. Symmetric to
    /// the read test: server writes via `async_write`, peer reads
    /// the bytes synchronously.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn async_write_end_to_end_via_io_engine() {
        let h = engine_new(8);
        assert!(h > 0);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        // Background: connect + read 7 bytes.
        let bg = std::thread::spawn(move || {
            use std::io::Read;
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            let mut got = [0u8; 7];
            s.read_exact(&mut got).unwrap();
            assert_eq!(&got, b"VERUM!\n");
        });

        let (stream, _) = listener.accept().unwrap();
        let server_fd = stream.as_raw_fd() as i64;
        std::mem::forget(stream);

        let payload: &[u8] = b"VERUM!\n";
        let n = async_write(
            h,
            server_fd,
            payload.as_ptr() as i64,
            payload.len() as i64,
            1_000_000_000, // 1s
        );
        assert_eq!(n, 7, "expected 7 bytes written, got {n}");

        bg.join().unwrap();
        unsafe { libc::close(server_fd as libc::c_int) };
        assert_eq!(engine_destroy(h), 0);
    }

    /// VBC-IO-ENGINE-2 — `async_read` with `len = 0` short-
    /// circuits to 0 without touching the engine. Pin the
    /// trivial-case behaviour.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn async_read_zero_len_returns_zero() {
        let h = engine_new(2);
        // Use any positive fd — won't be touched.
        let n = async_read(h, 99999, 0x1000_0000, 0, 100_000_000);
        assert_eq!(n, 0);
        assert_eq!(engine_destroy(h), 0);
    }

    /// VBC-IO-ENGINE-2 — bad inputs (negative fd / null buffer /
    /// negative length) surface as IO_ERROR.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn async_read_bad_inputs_return_io_error() {
        use super::super::dispatch_table::handlers::net_runtime::NET_STATUS_IO_ERROR;
        let h = engine_new(2);
        assert_eq!(
            async_read(h, -1, 0x1000_0000, 8, 100_000_000),
            NET_STATUS_IO_ERROR,
            "negative fd"
        );
        assert_eq!(
            async_read(h, 1, 0, 8, 100_000_000),
            NET_STATUS_IO_ERROR,
            "null buffer"
        );
        assert_eq!(
            async_read(h, 1, 0x1000_0000, -1, 100_000_000),
            NET_STATUS_IO_ERROR,
            "negative length"
        );
        assert_eq!(engine_destroy(h), 0);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn distinct_sessions_independent() {
        let h1 = engine_new(8);
        let h2 = engine_new(8);
        assert!(h1 > 0 && h2 > 0 && h1 != h2);
        let l1 = TcpListener::bind("127.0.0.1:0").unwrap();
        let l2 = TcpListener::bind("127.0.0.1:0").unwrap();
        l1.set_nonblocking(true).unwrap();
        l2.set_nonblocking(true).unwrap();
        let fd1 = l1.as_raw_fd() as i64;
        let fd2 = l2.as_raw_fd() as i64;
        assert_eq!(submit(h1, fd1, FLAG_READ as i64), 0);
        assert_eq!(submit(h2, fd2, FLAG_READ as i64), 0);
        // Submitting fd1 to h2 doesn't affect h1's state.
        let n1 = poll(h1, 4, 50_000_000);
        let n2 = poll(h2, 4, 50_000_000);
        assert_eq!(n1, 0);
        assert_eq!(n2, 0);
        assert_eq!(engine_destroy(h1), 0);
        assert_eq!(engine_destroy(h2), 0);
    }
}
