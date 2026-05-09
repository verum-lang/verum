//! Single source of truth for OS-level event-loop constants.
//!
//! Unlike errno / sockets / file flags / mmap (where Linux and Darwin
//! use the same API with different numeric values), event-loop
//! infrastructure is **structurally different** per platform:
//!
//! * **Darwin**: kqueue / kevent — `EVFILT_*` filter type IDs (negative)
//!   and `EV_*` action / behavior flags (positive bitmasks). API
//!   centred on `kqueue()` / `kevent()` syscalls.
//!
//! * **Linux**: epoll — `EPOLL_CTL_*` control commands and `EPOLL*`
//!   event-bit flags. API centred on `epoll_create1()` / `epoll_ctl()` /
//!   `epoll_pwait()` syscalls.
//!
//! * **Windows**: IOCP (I/O Completion Ports) — out of scope for this
//!   module; named here for reference.
//!
//! Since the two APIs are not drop-in compatible, **there are no
//! cross-platform constants** in this module. Each canonical name
//! exists in exactly one of `kqueue` (Darwin) / `epoll` (Linux), and
//! the dispatch helper returns `None` for cross-target requests
//! (`EVFILT_READ` on Linux, `EPOLLIN` on Darwin).
//!
//! Authoritative declarations live in:
//! * `core/sys/darwin/libsystem.vr` — kqueue side
//! * `core/sys/linux/io.vr` — epoll side
//!
//! Pre-this-module, the codegen `intrinsic_constant_value` table hard-
//! coded only the Darwin kqueue values. Linux programs referencing
//! `@const EPOLLIN` got nothing back; programs referencing
//! `@const EVFILT_READ` on Linux silently got a Darwin value with no
//! Linux semantics — a third-class platform-misclassification bug.

// ============================================================================
// Darwin kqueue / kevent constants
// ============================================================================

/// Darwin kqueue filter type IDs and event flags per
/// `<sys/event.h>` and `core/sys/darwin/libsystem.vr`.
pub mod kqueue {
    /// Filter type: file descriptor readable.
    pub const EVFILT_READ: i64 = -1;
    /// Filter type: file descriptor writable.
    pub const EVFILT_WRITE: i64 = -2;
    /// Filter type: AIO completion (Darwin-specific).
    pub const EVFILT_AIO: i64 = -3;
    /// Filter type: vnode events (file changes).
    pub const EVFILT_VNODE: i64 = -4;
    /// Filter type: process state.
    pub const EVFILT_PROC: i64 = -5;
    /// Filter type: signal delivery.
    pub const EVFILT_SIGNAL: i64 = -6;
    /// Filter type: timer expiry.
    pub const EVFILT_TIMER: i64 = -7;
    /// Filter type: Mach port (Darwin-specific).
    pub const EVFILT_MACHPORT: i64 = -8;
    /// Filter type: filesystem state.
    pub const EVFILT_FS: i64 = -9;
    /// Filter type: user-defined trigger.
    pub const EVFILT_USER: i64 = -10;

    /// kevent action: add event to kqueue.
    pub const EV_ADD: i64 = 0x0001;
    /// kevent action: remove event from kqueue.
    pub const EV_DELETE: i64 = 0x0002;
    /// kevent action: enable a previously-disabled event.
    pub const EV_ENABLE: i64 = 0x0004;
    /// kevent action: disable an event without removing it.
    pub const EV_DISABLE: i64 = 0x0008;
    /// kevent behavior: edge-triggered (clear after delivery).
    pub const EV_CLEAR: i64 = 0x0020;
    /// kevent behavior: deliver only once then auto-disable.
    pub const EV_ONESHOT: i64 = 0x0010;
    /// kevent flag: end-of-file reached.
    pub const EV_EOF: i64 = 0x8000;
    /// kevent flag: error from kernel.
    pub const EV_ERROR: i64 = 0x4000;
}

// ============================================================================
// Linux epoll constants
// ============================================================================

/// Linux epoll control commands and event bits per
/// `<sys/epoll.h>` and `core/sys/linux/io.vr`.
pub mod epoll {
    /// `epoll_ctl()` operation: register file descriptor.
    pub const EPOLL_CTL_ADD: i64 = 1;
    /// `epoll_ctl()` operation: deregister file descriptor.
    pub const EPOLL_CTL_DEL: i64 = 2;
    /// `epoll_ctl()` operation: modify registration.
    pub const EPOLL_CTL_MOD: i64 = 3;

    /// Event bit: file descriptor readable.
    pub const EPOLLIN: i64 = 0x001;
    /// Event bit: priority data readable.
    pub const EPOLLPRI: i64 = 0x002;
    /// Event bit: file descriptor writable.
    pub const EPOLLOUT: i64 = 0x004;
    /// Event bit: error condition.
    pub const EPOLLERR: i64 = 0x008;
    /// Event bit: file descriptor hung up.
    pub const EPOLLHUP: i64 = 0x010;
    /// Event bit: peer closed connection (Linux 2.6.17+).
    pub const EPOLLRDHUP: i64 = 0x2000;
    /// Event behavior: edge-triggered.
    pub const EPOLLET: i64 = 1 << 31;
    /// Event behavior: deliver only once then auto-disable.
    pub const EPOLLONESHOT: i64 = 1 << 30;
    /// Event behavior: wake CPU even from system suspend.
    pub const EPOLLWAKEUP: i64 = 1 << 29;
    /// Event behavior: file descriptor will be closed on exec().
    pub const EPOLLEXCLUSIVE: i64 = 1 << 28;
}

// ============================================================================
// Target-conditional dispatch
// ============================================================================

/// Resolve a kqueue or epoll constant by name and target OS.
/// Returns `None` if the name doesn't exist for the given target
/// (e.g., `EPOLLIN` on Darwin, `EVFILT_READ` on Linux) — the caller
/// must guard cross-platform code with `@cfg(target_os = ...)`.
pub fn os_event_const_for_target(name: &str, target_os: &str) -> Option<i64> {
    let is_darwin = matches!(target_os, "macos" | "darwin" | "ios" | "tvos" | "watchos");
    let is_linux = target_os == "linux";

    if is_darwin {
        match name {
            "EVFILT_READ" => Some(kqueue::EVFILT_READ),
            "EVFILT_WRITE" => Some(kqueue::EVFILT_WRITE),
            "EVFILT_AIO" => Some(kqueue::EVFILT_AIO),
            "EVFILT_VNODE" => Some(kqueue::EVFILT_VNODE),
            "EVFILT_PROC" => Some(kqueue::EVFILT_PROC),
            "EVFILT_SIGNAL" => Some(kqueue::EVFILT_SIGNAL),
            "EVFILT_TIMER" => Some(kqueue::EVFILT_TIMER),
            "EVFILT_MACHPORT" => Some(kqueue::EVFILT_MACHPORT),
            "EVFILT_FS" => Some(kqueue::EVFILT_FS),
            "EVFILT_USER" => Some(kqueue::EVFILT_USER),
            "EV_ADD" => Some(kqueue::EV_ADD),
            "EV_DELETE" => Some(kqueue::EV_DELETE),
            "EV_ENABLE" => Some(kqueue::EV_ENABLE),
            "EV_DISABLE" => Some(kqueue::EV_DISABLE),
            "EV_CLEAR" => Some(kqueue::EV_CLEAR),
            "EV_ONESHOT" => Some(kqueue::EV_ONESHOT),
            "EV_EOF" => Some(kqueue::EV_EOF),
            "EV_ERROR" => Some(kqueue::EV_ERROR),
            _ => None,
        }
    } else if is_linux {
        match name {
            "EPOLL_CTL_ADD" => Some(epoll::EPOLL_CTL_ADD),
            "EPOLL_CTL_DEL" => Some(epoll::EPOLL_CTL_DEL),
            "EPOLL_CTL_MOD" => Some(epoll::EPOLL_CTL_MOD),
            "EPOLLIN" => Some(epoll::EPOLLIN),
            "EPOLLPRI" => Some(epoll::EPOLLPRI),
            "EPOLLOUT" => Some(epoll::EPOLLOUT),
            "EPOLLERR" => Some(epoll::EPOLLERR),
            "EPOLLHUP" => Some(epoll::EPOLLHUP),
            "EPOLLRDHUP" => Some(epoll::EPOLLRDHUP),
            "EPOLLET" => Some(epoll::EPOLLET),
            "EPOLLONESHOT" => Some(epoll::EPOLLONESHOT),
            "EPOLLWAKEUP" => Some(epoll::EPOLLWAKEUP),
            "EPOLLEXCLUSIVE" => Some(epoll::EPOLLEXCLUSIVE),
            _ => None,
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Darwin kqueue values per `<sys/event.h>` and stdlib
    /// `core/sys/darwin/libsystem.vr`.
    #[test]
    fn kqueue_constants_pinned() {
        // Filter types are negative so they don't collide with
        // user-data ids in kevent struct.
        assert_eq!(kqueue::EVFILT_READ, -1);
        assert_eq!(kqueue::EVFILT_WRITE, -2);
        assert_eq!(kqueue::EVFILT_TIMER, -7);
        assert_eq!(kqueue::EVFILT_USER, -10);
        // Action / behavior flags are positive bitmasks.
        assert_eq!(kqueue::EV_ADD, 0x0001);
        assert_eq!(kqueue::EV_DELETE, 0x0002);
        assert_eq!(kqueue::EV_CLEAR, 0x0020);
        assert_eq!(kqueue::EV_ONESHOT, 0x0010);
        assert_eq!(kqueue::EV_EOF, 0x8000);
        assert_eq!(kqueue::EV_ERROR, 0x4000);
    }

    /// Linux epoll values per `<sys/epoll.h>` and stdlib
    /// `core/sys/linux/io.vr`.
    #[test]
    fn epoll_constants_pinned() {
        // Control commands.
        assert_eq!(epoll::EPOLL_CTL_ADD, 1);
        assert_eq!(epoll::EPOLL_CTL_DEL, 2);
        assert_eq!(epoll::EPOLL_CTL_MOD, 3);
        // Event bits — bit 0 / 2 / 3 / 4.
        assert_eq!(epoll::EPOLLIN, 0x001);
        assert_eq!(epoll::EPOLLPRI, 0x002);
        assert_eq!(epoll::EPOLLOUT, 0x004);
        assert_eq!(epoll::EPOLLERR, 0x008);
        assert_eq!(epoll::EPOLLHUP, 0x010);
        assert_eq!(epoll::EPOLLRDHUP, 0x2000);
        // Behavior bits — high bits of the 32-bit events word.
        assert_eq!(epoll::EPOLLET, 1 << 31);
        assert_eq!(epoll::EPOLLONESHOT, 1 << 30);
    }

    /// Cross-target API independence: kqueue names DON'T resolve on
    /// Linux; epoll names DON'T resolve on Darwin. Caller must guard
    /// with `@cfg(target_os = ...)`.
    #[test]
    fn kqueue_and_epoll_dont_cross_targets() {
        // kqueue → not on Linux.
        assert_eq!(os_event_const_for_target("EVFILT_READ", "linux"), None);
        assert_eq!(os_event_const_for_target("EV_ADD", "linux"), None);
        assert_eq!(os_event_const_for_target("EV_CLEAR", "linux"), None);

        // epoll → not on Darwin.
        assert_eq!(os_event_const_for_target("EPOLLIN", "macos"), None);
        assert_eq!(os_event_const_for_target("EPOLL_CTL_ADD", "darwin"), None);
        assert_eq!(os_event_const_for_target("EPOLLET", "ios"), None);

        // Neither resolves on Windows (no kqueue, no epoll — IOCP is
        // a different model not exposed here).
        assert_eq!(os_event_const_for_target("EVFILT_READ", "windows"), None);
        assert_eq!(os_event_const_for_target("EPOLLIN", "windows"), None);
    }

    /// Target dispatch correctly routes per-platform values.
    #[test]
    fn os_event_const_for_target_dispatch() {
        // Darwin kqueue dispatch.
        assert_eq!(os_event_const_for_target("EVFILT_READ", "macos"), Some(-1));
        assert_eq!(os_event_const_for_target("EVFILT_WRITE", "darwin"), Some(-2));
        assert_eq!(os_event_const_for_target("EV_ADD", "ios"), Some(0x0001));

        // Linux epoll dispatch.
        assert_eq!(os_event_const_for_target("EPOLLIN", "linux"), Some(0x001));
        assert_eq!(os_event_const_for_target("EPOLLOUT", "linux"), Some(0x004));
        assert_eq!(
            os_event_const_for_target("EPOLLET", "linux"),
            Some(1 << 31),
        );
        assert_eq!(
            os_event_const_for_target("EPOLL_CTL_ADD", "linux"),
            Some(1),
        );
    }
}
