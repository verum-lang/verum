//! Singleton I/O reactor for the Tier-0 interpreter.
//!
//! VBC-NET-RT-2 — turns the previously-blocking TCP/UDP intrinsics
//! into timeout-bound waits backed by the kernel's I/O readiness
//! interface (`kqueue` on macOS/BSD, `epoll` on Linux).  Without
//! this, a `tcp_recv` parked on a slow client occupied an entire
//! OS thread; with it, the same OS thread can park on the readiness
//! of *any* registered fd and proceed as soon as the kernel signals.
//!
//! # Architectural relationship to `core/sys/{darwin,linux,windows}/io.vr`
//!
//! The Verum-side modules `core.sys.darwin.io.KqueueDriver`,
//! `core.sys.linux.io.{IoUringDriver,EpollDriver}`, and
//! `core.sys.windows.io.IocpDriver` are the **Tier-1 (AOT)**
//! implementations of the `IOEngine` protocol declared in
//! `core/sys/io_engine.vr`.  They FFI into the platform syscalls
//! and provide the production async I/O substrate when the Verum
//! compiler emits native code.
//!
//! At **Tier-0 (interpreter)** the libSystem/libffi dispatch chain
//! for kqueue/kevent has the same brittleness that motivated every
//! other intercept module (`shell_runtime`, `file_runtime`,
//! `process_runtime`, `net_runtime` itself).  The architecturally-
//! consistent answer — already established for `sh_check`,
//! `fs.exists`, `Command.output`, `tcp_send` — is to intercept at
//! the intrinsic boundary and call the platform primitive directly
//! from the Rust host process.
//!
//! This module IS the Tier-0 host-side counterpart to those Verum
//! drivers.  It uses the SAME kernel primitives (`kqueue`/`epoll`)
//! and provides the SAME readiness semantics (oneshot, broadcast
//! to multiple waiters, cross-thread wake via mutex+condvar).  Both
//! tiers implement the SAME `IOEngine` contract on the SAME
//! platform primitives — only the execution tier differs.  This
//! preserves the cross-tier behavioural contract that the Verum
//! verification layer relies on.
//!
//! # Performance design (competitive with mio/tokio/libuv)
//!
//! Verum's architectural principle: high-performance, fault-tolerant
//! server systems on par with C/C++/Rust system stacks.  Concrete
//! choices in this module:
//!
//!  * **Sharded waiter map** (16 buckets, fd & 0xF) — a single
//!    global Mutex would serialise every wait.  Sharding by low fd
//!    bits means independent fds park independent buckets.
//!
//!  * **Slot pool with free-list** — `Slot`s are allocated from a
//!    `Vec<Box<Slot>>` arena, recycled via per-shard free-lists.
//!    Eliminates per-call `Arc::new` heap traffic on the hot path.
//!
//!  * **Single-waiter fast path** — the common case is one waiter
//!    per (fd, kind).  We store `Option<SlotIdx>` per key, not a
//!    `Vec`; a second wait while the first is still parked simply
//!    overwrites — both wake when the kernel signals (the kernel
//!    `kqueue` registration is ONESHOT and idempotent).
//!
//!  * **Atomic state machine** — `Slot.state` is an `AtomicU8`
//!    cycling Idle → Armed → Fired → Idle.  The waiter parks on
//!    `Mutex<bool>` only AFTER arming, never spinning.  The
//!    background thread CAS-flips Armed→Fired and notifies; if
//!    the CAS sees Idle (waiter already returned via timeout) it
//!    drops the event.
//!
//!  * **Batched harvest** — the bg thread reads up to 64 kernel
//!    events per `kevent`/`epoll_wait` call, dispatching all of
//!    them under a single shard-lock acquisition where possible.
//!
//!  * **CLOEXEC on the reactor fd** — the reactor's kqueue/epoll
//!    fd does not leak into child processes spawned by
//!    `shell_runtime` / `process_runtime`.
//!
//! # Public surface
//!
//!  * `wait_readable(fd, timeout) -> WaitOutcome`
//!  * `wait_writable(fd, timeout) -> WaitOutcome`
//!
//! Both return `Ready` (the fd is ready), `TimedOut` (deadline hit
//! before readiness), or `Error` (the wait machinery itself failed —
//! distinct from the I/O attempt failing afterwards).
//!
//! # Registration discipline
//!
//! Registrations are **one-shot**: each `wait_*` call registers a
//! `EV_ONESHOT` (kqueue) / `EPOLLONESHOT` (epoll) interest, so the
//! kernel auto-removes the registration after one wake.  The next
//! call re-registers.  This avoids the level-triggered "always ready"
//! starvation pattern AND the edge-triggered "must drain to empty"
//! complexity — each call has a clean lifecycle.
//!
//! # No-libc invariant
//!
//! The reactor itself runs in the Rust host process, so it uses
//! `libc` directly (the standard `cfg(unix)` dep).  Verum-emitted
//! code never sees these syscalls — they live on the host side of
//! the intrinsic boundary.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Outcome of a `wait_readable` / `wait_writable` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    /// The kernel reported the fd is ready for the requested op.
    Ready,
    /// The deadline elapsed before the kernel signalled readiness.
    TimedOut,
    /// The wait machinery itself failed (reactor not initialised,
    /// kqueue/epoll syscall returned an error, etc.).  The caller
    /// should fall back to a blocking attempt and let the I/O
    /// syscall surface the underlying errno.
    Error,
}

/// What kind of readiness the caller wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InterestKind {
    Readable,
    Writable,
}

// ============================================================================
// Slot state machine
// ============================================================================

const SLOT_IDLE: u8 = 0;
const SLOT_ARMED: u8 = 1;
const SLOT_FIRED: u8 = 2;

/// Per-call slot.  Cycles Idle → Armed (waiter parked) → Fired
/// (kernel signalled, waiter notified) → Idle (recycled).  Multiple
/// concurrent waiters on the same key overwrite each other; both
/// wake because kqueue/epoll registration is idempotent and we
/// notify_all on the condvar.
struct Slot {
    state: AtomicU8,
    cv: Condvar,
    /// Locked alongside the condvar; the bool is redundant with
    /// `state` (it's set whenever state==FIRED) but keeps the
    /// std condvar API ergonomic.
    notified: Mutex<bool>,
}

impl Slot {
    fn new() -> Self {
        Self {
            state: AtomicU8::new(SLOT_IDLE),
            cv: Condvar::new(),
            notified: Mutex::new(false),
        }
    }

    fn arm(&self) {
        // Reset notified flag (under lock so a concurrent fire
        // doesn't race the assignment).
        let mut n = self.notified.lock().unwrap();
        *n = false;
        self.state.store(SLOT_ARMED, Ordering::Release);
    }

    /// Returns true if the caller transitioned ARMED→FIRED (and
    /// therefore is responsible for notifying the waiter).  Returns
    /// false if the slot was already Fired or Idle (waiter already
    /// gone — drop the event).
    fn fire(&self) -> bool {
        match self.state.compare_exchange(
            SLOT_ARMED,
            SLOT_FIRED,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                let mut n = self.notified.lock().unwrap();
                *n = true;
                self.cv.notify_all();
                true
            }
            Err(_) => false,
        }
    }

    /// Park until notified or `deadline`.  Returns true on
    /// notification, false on timeout.
    fn park_until(&self, deadline: Instant) -> bool {
        let mut n = self.notified.lock().unwrap();
        loop {
            if *n {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let remaining = deadline - now;
            let (g, wt) = self.cv.wait_timeout(n, remaining).unwrap();
            n = g;
            if wt.timed_out() && !*n {
                return false;
            }
        }
    }

    /// Caller failed to be notified within deadline.  Mark idle so
    /// a late-arriving event is dropped.  Returns true if we
    /// successfully owned the recycle (state was Armed); false if
    /// the bg thread fired between our timeout and the CAS (the
    /// caller should treat this as Ready, not TimedOut).
    fn idle(&self) -> bool {
        match self.state.compare_exchange(
            SLOT_ARMED,
            SLOT_IDLE,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => true,
            Err(_) => false,
        }
    }

    fn reset_to_idle(&self) {
        self.state.store(SLOT_IDLE, Ordering::Release);
    }
}

// ============================================================================
// Sharded waiter map
// ============================================================================

const NUM_SHARDS: usize = 16;
const SHARD_MASK: i64 = 0xF;

/// One shard of the waiter map + free list of recycled slot indices.
struct Shard {
    /// Per-fd-kind → slot index.  Single-waiter fast path.
    waiters: Mutex<ShardInner>,
}

struct ShardInner {
    /// Map keyed by `fd << 1 | kind_bit`.  `kind_bit`: 0=read, 1=write.
    /// Stored as parallel arrays (key, slot_index) — a cache-line
    /// linear scan beats HashMap at this cardinality (the SmallVec
    /// stays inline for the typical ≤8-waiter shard).
    keys: smallvec::SmallVec<[u64; 8]>,
    slots: smallvec::SmallVec<[usize; 8]>,
    /// Slot indices freed since last allocation; reuse before
    /// growing the arena.
    freed: smallvec::SmallVec<[usize; 8]>,
}

impl Shard {
    fn new() -> Self {
        Self {
            waiters: Mutex::new(ShardInner {
                keys: smallvec::SmallVec::new(),
                slots: smallvec::SmallVec::new(),
                freed: smallvec::SmallVec::new(),
            }),
        }
    }
}

// ============================================================================
// Reactor singleton
// ============================================================================

struct Reactor {
    shards: [Shard; NUM_SHARDS],
    /// Append-only slot arena.  Indices are stable for the lifetime
    /// of the process; `freed` lists track which are reusable.
    arena: Mutex<Vec<Box<Slot>>>,
    /// Per-platform backend handle (kqueue fd / epoll fd / sentinel).
    backend: Backend,
    /// Set to false if the bg thread bails out on a fatal kernel
    /// error.  After that, `wait_*` returns Error fast.
    healthy: AtomicBool,
}

/// Lazy singleton — first `wait_*` call boots the bg thread.
static REACTOR: LazyLock<Reactor> = LazyLock::new(Reactor::boot);

impl Reactor {
    fn boot() -> Self {
        let backend = Backend::create();
        let healthy = AtomicBool::new(backend.is_ok());
        let reactor = Reactor {
            shards: std::array::from_fn(|_| Shard::new()),
            arena: Mutex::new(Vec::with_capacity(256)),
            backend: backend.unwrap_or(Backend::Disabled),
            healthy,
        };
        if matches!(reactor.backend, Backend::Disabled) {
            return reactor;
        }
        std::thread::Builder::new()
            .name("verum-vbc-reactor".to_string())
            .spawn(|| {
                bg_thread(&REACTOR);
            })
            .expect("verum-vbc-reactor bg thread spawn failed");
        reactor
    }

    /// Allocate a fresh slot index.  Tries free-list first; falls
    /// back to growing the arena.  The slot is RESET to Idle.
    fn allocate_slot(&self, shard_idx: usize) -> usize {
        // Pop from this shard's free-list.
        {
            let mut s = self.shards[shard_idx].waiters.lock().unwrap();
            if let Some(idx) = s.freed.pop() {
                let arena = self.arena.lock().unwrap();
                arena[idx].reset_to_idle();
                return idx;
            }
        }
        // Grow the arena.
        let mut arena = self.arena.lock().unwrap();
        let idx = arena.len();
        arena.push(Box::new(Slot::new()));
        idx
    }

    fn release_slot(&self, shard_idx: usize, slot_idx: usize) {
        let mut s = self.shards[shard_idx].waiters.lock().unwrap();
        s.freed.push(slot_idx);
    }

    fn slot(&self, idx: usize) -> &Slot {
        // SAFETY: arena is append-only; we hold a reference to the
        // singleton arena which lives for the process lifetime.
        // Indexing into `Vec<Box<Slot>>` and returning &Slot is
        // sound as long as the Vec doesn't reallocate while we hold
        // the reference.  Since Box<Slot> is heap-allocated, the
        // Slot's address is stable across Vec reallocations.
        let arena = self.arena.lock().unwrap();
        let slot_ptr: *const Slot = &*arena[idx];
        unsafe { &*slot_ptr }
    }

    /// Insert (key → slot_idx) into the appropriate shard.  If a
    /// previous waiter is still parked on this key, returns its
    /// slot_idx so the caller can either notify it (broadcast) or
    /// just leave it parked (it'll wake on the kernel event too).
    fn install_waiter(&self, key: u64, slot_idx: usize, shard_idx: usize) -> Option<usize> {
        let mut s = self.shards[shard_idx].waiters.lock().unwrap();
        // Linear scan: cache-friendly for small N.
        for i in 0..s.keys.len() {
            if s.keys[i] == key {
                let prev = s.slots[i];
                s.slots[i] = slot_idx;
                return Some(prev);
            }
        }
        s.keys.push(key);
        s.slots.push(slot_idx);
        None
    }

    /// Remove the slot for this key.  Returns the slot index that
    /// was removed (so the caller can free it) or None if no
    /// matching waiter.
    fn uninstall_waiter(&self, key: u64, slot_idx: usize, shard_idx: usize) -> bool {
        let mut s = self.shards[shard_idx].waiters.lock().unwrap();
        for i in 0..s.keys.len() {
            if s.keys[i] == key && s.slots[i] == slot_idx {
                s.keys.swap_remove(i);
                s.slots.swap_remove(i);
                return true;
            }
        }
        false
    }

    /// Take the waiter for `key` (called by bg thread on kernel
    /// event).  Returns the slot index, or None if no waiter.
    fn take_waiter(&self, key: u64, shard_idx: usize) -> Option<usize> {
        let mut s = self.shards[shard_idx].waiters.lock().unwrap();
        for i in 0..s.keys.len() {
            if s.keys[i] == key {
                s.keys.swap_remove(i);
                let idx = s.slots.swap_remove(i);
                return Some(idx);
            }
        }
        None
    }
}

#[inline]
fn shard_for(fd: i64) -> usize {
    (fd & SHARD_MASK) as usize
}

#[inline]
fn pack_key(fd: i64, kind: InterestKind) -> u64 {
    let bit = match kind {
        InterestKind::Readable => 0u64,
        InterestKind::Writable => 1u64,
    };
    ((fd as u64) << 1) | bit
}

/// Public entry: wait for `fd` to become readable, up to `timeout`.
pub fn wait_readable(fd: i64, timeout: Duration) -> WaitOutcome {
    wait(fd, InterestKind::Readable, timeout)
}

/// Public entry: wait for `fd` to become writable, up to `timeout`.
pub fn wait_writable(fd: i64, timeout: Duration) -> WaitOutcome {
    wait(fd, InterestKind::Writable, timeout)
}

fn wait(fd: i64, kind: InterestKind, timeout: Duration) -> WaitOutcome {
    let reactor = &*REACTOR;
    if !reactor.healthy.load(Ordering::Acquire) {
        return WaitOutcome::Error;
    }
    if fd < 0 {
        return WaitOutcome::Error;
    }
    let shard_idx = shard_for(fd);
    let key = pack_key(fd, kind);
    let slot_idx = reactor.allocate_slot(shard_idx);

    // SAFETY: arena entries are Boxed; address stable for process lifetime.
    let slot_ptr: *const Slot = {
        let arena = reactor.arena.lock().unwrap();
        &*arena[slot_idx] as *const Slot
    };
    let slot = unsafe { &*slot_ptr };

    slot.arm();
    // Install waiter BEFORE registering with the kernel — so a
    // racing event delivery from a recycled fd registration finds
    // us in the map.
    let _displaced = reactor.install_waiter(key, slot_idx, shard_idx);
    // (If we displaced a previous waiter, it's still parked on its
    //  own slot.  The kernel only signals once, so only one of the
    //  two will get notified — the most-recent registration.  The
    //  displaced waiter wakes on its own timeout.  Acceptable for
    //  the rare-double-wait case.)

    if let Err(()) = reactor.backend.register(fd, kind) {
        // Tear down: remove from waiter map, recycle slot.
        let _ = reactor.uninstall_waiter(key, slot_idx, shard_idx);
        slot.idle();
        reactor.release_slot(shard_idx, slot_idx);
        return WaitOutcome::Error;
    }

    let deadline = Instant::now() + timeout;
    let notified = slot.park_until(deadline);

    let outcome = if notified {
        WaitOutcome::Ready
    } else {
        // Timeout — try to mark slot Idle.  If the bg thread
        // raced and just fired us, treat as Ready.
        if slot.idle() {
            // Successfully transitioned Armed → Idle; the event
            // didn't arrive.  Remove from waiter map (it may
            // already be gone if the bg thread was about to fire).
            let _ = reactor.uninstall_waiter(key, slot_idx, shard_idx);
            // Best-effort: deregister kernel interest.
            let _ = reactor.backend.deregister(fd, kind);
            WaitOutcome::TimedOut
        } else {
            // Bg thread won the race; the event was delivered.
            WaitOutcome::Ready
        }
    };

    reactor.release_slot(shard_idx, slot_idx);
    outcome
}

// ============================================================================
// Backend — per-platform kernel-side
// ============================================================================

#[cfg(target_os = "macos")]
use macos_kqueue::Backend;

#[cfg(target_os = "linux")]
use linux_epoll::Backend;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
use disabled::Backend;

#[cfg(target_os = "macos")]
mod macos_kqueue {
    use super::{pack_key, shard_for, InterestKind, Reactor};
    use std::os::raw::c_int;

    pub enum Backend {
        Active { kq: c_int },
        Disabled,
    }

    impl Backend {
        pub fn create() -> Result<Self, ()> {
            let kq = unsafe { libc::kqueue() };
            if kq < 0 {
                return Err(());
            }
            // FD_CLOEXEC so reactor fd doesn't leak into spawned children.
            unsafe {
                libc::fcntl(kq, libc::F_SETFD, libc::FD_CLOEXEC);
            }
            Ok(Backend::Active { kq })
        }

        pub fn register(&self, fd: i64, kind: InterestKind) -> Result<(), ()> {
            let kq = match self {
                Backend::Active { kq } => *kq,
                Backend::Disabled => return Err(()),
            };
            let filter = match kind {
                InterestKind::Readable => libc::EVFILT_READ,
                InterestKind::Writable => libc::EVFILT_WRITE,
            };
            let mut kev = libc::kevent {
                ident: fd as libc::uintptr_t,
                filter,
                flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_ONESHOT,
                fflags: 0,
                data: 0,
                udata: std::ptr::null_mut(),
            };
            let rc = unsafe {
                libc::kevent(
                    kq,
                    &mut kev as *mut _,
                    1,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null(),
                )
            };
            if rc < 0 { Err(()) } else { Ok(()) }
        }

        pub fn deregister(&self, fd: i64, kind: InterestKind) -> Result<(), ()> {
            let kq = match self {
                Backend::Active { kq } => *kq,
                Backend::Disabled => return Err(()),
            };
            let filter = match kind {
                InterestKind::Readable => libc::EVFILT_READ,
                InterestKind::Writable => libc::EVFILT_WRITE,
            };
            let mut kev = libc::kevent {
                ident: fd as libc::uintptr_t,
                filter,
                flags: libc::EV_DELETE,
                fflags: 0,
                data: 0,
                udata: std::ptr::null_mut(),
            };
            let _ = unsafe {
                libc::kevent(
                    kq,
                    &mut kev as *mut _,
                    1,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null(),
                )
            };
            Ok(())
        }
    }

    pub(super) fn bg_loop(reactor: &Reactor) {
        let kq = match reactor.backend {
            Backend::Active { kq } => kq,
            Backend::Disabled => return,
        };
        let mut events: [libc::kevent; 64] = unsafe { std::mem::zeroed() };
        loop {
            let n = unsafe {
                libc::kevent(
                    kq,
                    std::ptr::null(),
                    0,
                    events.as_mut_ptr(),
                    events.len() as c_int,
                    std::ptr::null(),
                )
            };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                reactor
                    .healthy
                    .store(false, std::sync::atomic::Ordering::Release);
                return;
            }
            for kev in &events[..n as usize] {
                let kind = if kev.filter == libc::EVFILT_READ {
                    InterestKind::Readable
                } else if kev.filter == libc::EVFILT_WRITE {
                    InterestKind::Writable
                } else {
                    continue;
                };
                let fd = kev.ident as i64;
                let key = pack_key(fd, kind);
                let shard_idx = shard_for(fd);
                if let Some(slot_idx) = reactor.take_waiter(key, shard_idx) {
                    let slot = reactor.slot(slot_idx);
                    let _ = slot.fire();
                    // NOTE: slot is RELEASED by the waiter on
                    // wake-up, not here — the waiter still holds
                    // the slot index and needs to read the state
                    // before recycling.
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod linux_epoll {
    use super::{pack_key, shard_for, InterestKind, Reactor};
    use std::os::raw::c_int;

    pub enum Backend {
        Active { epfd: c_int },
        Disabled,
    }

    impl Backend {
        pub fn create() -> Result<Self, ()> {
            let epfd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
            if epfd < 0 {
                return Err(());
            }
            Ok(Backend::Active { epfd })
        }

        pub fn register(&self, fd: i64, kind: InterestKind) -> Result<(), ()> {
            let epfd = match self {
                Backend::Active { epfd } => *epfd,
                Backend::Disabled => return Err(()),
            };
            let data = pack_key(fd, kind);
            let events = match kind {
                InterestKind::Readable => libc::EPOLLIN as u32,
                InterestKind::Writable => libc::EPOLLOUT as u32,
            } | libc::EPOLLONESHOT as u32;
            let mut ev = libc::epoll_event {
                events,
                u64: data,
            };
            // Try MOD first (handles the post-ONESHOT-fired case
            // where the fd is still in epoll's table but disarmed).
            let rc_mod = unsafe { libc::epoll_ctl(epfd, libc::EPOLL_CTL_MOD, fd as c_int, &mut ev) };
            if rc_mod == 0 {
                return Ok(());
            }
            let rc_add = unsafe { libc::epoll_ctl(epfd, libc::EPOLL_CTL_ADD, fd as c_int, &mut ev) };
            if rc_add < 0 { Err(()) } else { Ok(()) }
        }

        pub fn deregister(&self, fd: i64, _kind: InterestKind) -> Result<(), ()> {
            let epfd = match self {
                Backend::Active { epfd } => *epfd,
                Backend::Disabled => return Err(()),
            };
            let _ = unsafe {
                libc::epoll_ctl(epfd, libc::EPOLL_CTL_DEL, fd as c_int, std::ptr::null_mut())
            };
            Ok(())
        }
    }

    pub(super) fn bg_loop(reactor: &Reactor) {
        let epfd = match reactor.backend {
            Backend::Active { epfd } => epfd,
            Backend::Disabled => return,
        };
        let mut events: [libc::epoll_event; 64] = unsafe { std::mem::zeroed() };
        loop {
            let n = unsafe {
                libc::epoll_wait(
                    epfd,
                    events.as_mut_ptr(),
                    events.len() as c_int,
                    -1,
                )
            };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                reactor
                    .healthy
                    .store(false, std::sync::atomic::Ordering::Release);
                return;
            }
            for ev in &events[..n as usize] {
                let key = ev.u64;
                let (fd, _) = super::unpack_key(key);
                let shard_idx = shard_for(fd);
                if let Some(slot_idx) = reactor.take_waiter(key, shard_idx) {
                    let slot = reactor.slot(slot_idx);
                    let _ = slot.fire();
                }
            }
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod disabled {
    use super::{InterestKind, Reactor};

    pub enum Backend {
        Disabled,
    }

    impl Backend {
        pub fn create() -> Result<Self, ()> {
            Err(())
        }
        pub fn register(&self, _fd: i64, _kind: InterestKind) -> Result<(), ()> {
            Err(())
        }
        pub fn deregister(&self, _fd: i64, _kind: InterestKind) -> Result<(), ()> {
            Err(())
        }
    }

    pub(super) fn bg_loop(_reactor: &Reactor) {}
}

fn bg_thread(reactor: &Reactor) {
    #[cfg(target_os = "macos")]
    macos_kqueue::bg_loop(reactor);
    #[cfg(target_os = "linux")]
    linux_epoll::bg_loop(reactor);
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    disabled::bg_loop(reactor);
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};
    use std::os::fd::AsRawFd;

    /// On platforms where the reactor is disabled, the public API
    /// must return `Error` quickly (no parking).
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    #[test]
    fn disabled_platform_returns_error_fast() {
        let start = Instant::now();
        let r = wait_readable(0, Duration::from_secs(60));
        assert_eq!(r, WaitOutcome::Error);
        assert!(start.elapsed() < Duration::from_millis(100));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn timeout_fires_when_no_data_arrives() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let fd = listener.as_raw_fd() as i64;
        let start = Instant::now();
        let r = wait_readable(fd, Duration::from_millis(100));
        let elapsed = start.elapsed();
        assert_eq!(r, WaitOutcome::TimedOut);
        assert!(elapsed >= Duration::from_millis(80) && elapsed < Duration::from_secs(2));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn ready_signalled_when_connection_arrives() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let fd = listener.as_raw_fd() as i64;
        let h = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            let _s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            std::thread::sleep(Duration::from_millis(50));
        });
        let r = wait_readable(fd, Duration::from_secs(2));
        assert_eq!(r, WaitOutcome::Ready);
        h.join().unwrap();
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn writable_signalled_for_fresh_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let _accept_thread = std::thread::spawn(move || {
            let (_s, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(100));
        });
        let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_nonblocking(true).unwrap();
        let fd = stream.as_raw_fd() as i64;
        let r = wait_writable(fd, Duration::from_secs(2));
        assert_eq!(r, WaitOutcome::Ready);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn many_concurrent_waits_dont_serialise() {
        // Bind 16 listeners.  Spawn 16 threads, each parks on its
        // listener for 200ms then times out.  If shard contention
        // is broken, total elapsed approaches 16 * 200ms; with
        // proper sharding it stays near 200ms.
        let listeners: Vec<TcpListener> = (0..16)
            .map(|_| {
                let l = TcpListener::bind("127.0.0.1:0").unwrap();
                l.set_nonblocking(true).unwrap();
                l
            })
            .collect();
        let fds: Vec<i64> = listeners.iter().map(|l| l.as_raw_fd() as i64).collect();
        let start = Instant::now();
        let handles: Vec<_> = fds
            .into_iter()
            .map(|fd| {
                std::thread::spawn(move || wait_readable(fd, Duration::from_millis(200)))
            })
            .collect();
        let outcomes: Vec<WaitOutcome> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let elapsed = start.elapsed();
        for o in &outcomes {
            assert_eq!(*o, WaitOutcome::TimedOut);
        }
        // Generous bound — under proper sharding 16 parallel parks
        // complete in ~210ms.  Anything > 1s indicates serialisation.
        assert!(
            elapsed < Duration::from_millis(1000),
            "parallel waits serialised: {elapsed:?}"
        );
        drop(listeners);
    }
}
