//! Single source of truth for POSIX socket constants.
//!
//! Authoritative declarations live in the Verum stdlib at
//! `core/sys/darwin/libsystem.vr` and `core/sys/linux/syscall.vr`.
//! Like POSIX errno, socket constants split into:
//!
//! * **Cross-platform**: `AF_INET = 2`, `SOCK_STREAM = 1`,
//!   `SOCK_DGRAM = 2`, `IPPROTO_TCP = 6`, `TCP_NODELAY = 1`,
//!   `SHUT_RD = 0`, `SHUT_WR = 1`, `SHUT_RDWR = 2`, `MSG_PEEK = 2`
//!   — agreed bit-for-bit on Linux + Darwin.
//!
//! * **Platform-divergent**: `AF_INET6`, `SOL_SOCKET`, `SO_REUSEADDR`,
//!   `SO_KEEPALIVE`, `SO_ERROR`, `MSG_DONTWAIT` — Linux and Darwin
//!   pick different numeric values for the same logical constant.
//!
//! | Constant       | Linux  | Darwin   |
//! |----------------|--------|----------|
//! | `AF_INET6`     |     10 |       30 |
//! | `SOL_SOCKET`   |      1 | `0xFFFF` |
//! | `SO_REUSEADDR` |      2 |        4 |
//! | `SO_KEEPALIVE` |      9 |        8 |
//! | `SO_ERROR`     |      4 | `0x1007` |
//! | `MSG_DONTWAIT` |   0x40 |     0x80 |
//!
//! Pre-this-module, `verum_vbc::codegen::mod.rs` carried a single
//! constants-resolution table that mixed Linux and Darwin assignments
//! (e.g., `AF_INET6 = 30` (Darwin), `SO_REUSEADDR = 2` (Linux),
//! `SO_KEEPALIVE = 8` (Darwin)) — the same platform-misclassification
//! bug class flagged for errno. Programs compiled for one target
//! receive constants from the other, leading to socket calls that
//! fail with `EINVAL`. This module exposes both platforms' values
//! as named submodules; `for_target(name, target_os)` dispatches.

// ============================================================================
// Cross-platform POSIX socket constants
// ============================================================================
//
// These nine constants are bit-identical on Linux + Darwin.

/// IPv4 address family (`AF_INET`).
pub const AF_INET: i64 = 2;
/// TCP-like stream socket type (`SOCK_STREAM`).
pub const SOCK_STREAM: i64 = 1;
/// UDP-like datagram socket type (`SOCK_DGRAM`).
pub const SOCK_DGRAM: i64 = 2;
/// TCP protocol number (`IPPROTO_TCP`).
pub const IPPROTO_TCP: i64 = 6;
/// Disable Nagle's algorithm (`TCP_NODELAY`).
pub const TCP_NODELAY: i64 = 1;
/// Shutdown receive direction (`SHUT_RD`).
pub const SHUT_RD: i64 = 0;
/// Shutdown send direction (`SHUT_WR`).
pub const SHUT_WR: i64 = 1;
/// Shutdown both directions (`SHUT_RDWR`).
pub const SHUT_RDWR: i64 = 2;
/// Peek at received data without removing it from the queue (`MSG_PEEK`).
pub const MSG_PEEK: i64 = 2;

// ============================================================================
// Platform-specific socket constants
// ============================================================================

/// Linux socket constant values (per `<bits/socket.h>` and
/// `core/sys/linux/syscall.vr`).
pub mod linux {
    pub const AF_INET6: i64 = 10;
    pub const SOL_SOCKET: i64 = 1;
    pub const SO_REUSEADDR: i64 = 2;
    pub const SO_KEEPALIVE: i64 = 9;
    pub const SO_ERROR: i64 = 4;
    pub const MSG_DONTWAIT: i64 = 0x40;
    pub const MSG_WAITALL: i64 = 0x100;
    pub const SOCK_NONBLOCK: i64 = 0o4000;
    pub const SOCK_CLOEXEC: i64 = 0o2000000;
    /// SOL_TCP exists on Linux (= 6, alias of IPPROTO_TCP).
    pub const SOL_TCP: i64 = 6;
}

/// Darwin (macOS / iOS / tvOS) socket constant values (per
/// `<sys/socket.h>` and `core/sys/darwin/libsystem.vr`).
pub mod darwin {
    pub const AF_INET6: i64 = 30;
    pub const SOL_SOCKET: i64 = 0xFFFF;
    pub const SO_REUSEADDR: i64 = 0x0004;
    pub const SO_KEEPALIVE: i64 = 0x0008;
    pub const SO_ERROR: i64 = 0x1007;
    pub const MSG_DONTWAIT: i64 = 0x80;
    /// MSG_WAITALL on Darwin (note: 0x40, *different* from Linux 0x100).
    pub const MSG_WAITALL: i64 = 0x40;
    // Darwin doesn't expose SOCK_NONBLOCK / SOCK_CLOEXEC as socket
    // type flags — uses fcntl(F_SETFL) instead. These are absent.
}

// ============================================================================
// Target-conditional dispatch
// ============================================================================

/// Resolve a platform-divergent socket constant by canonical name and
/// target-OS string. Returns `None` for cross-platform names (use the
/// module-level constants), unknown names, or unknown targets.
///
/// Used by codegen when emitting target-specific `@const AF_INET6`-
/// style references — the build target dictates the value.
pub fn socket_const_for_target(name: &str, target_os: &str) -> Option<i64> {
    let is_darwin = matches!(target_os, "macos" | "darwin" | "ios" | "tvos" | "watchos");
    let is_linux = target_os == "linux";
    if !is_darwin && !is_linux {
        return None;
    }
    match name {
        "AF_INET6" => Some(if is_darwin {
            darwin::AF_INET6
        } else {
            linux::AF_INET6
        }),
        "SOL_SOCKET" => Some(if is_darwin {
            darwin::SOL_SOCKET
        } else {
            linux::SOL_SOCKET
        }),
        "SO_REUSEADDR" => Some(if is_darwin {
            darwin::SO_REUSEADDR
        } else {
            linux::SO_REUSEADDR
        }),
        "SO_KEEPALIVE" => Some(if is_darwin {
            darwin::SO_KEEPALIVE
        } else {
            linux::SO_KEEPALIVE
        }),
        "SO_ERROR" => Some(if is_darwin {
            darwin::SO_ERROR
        } else {
            linux::SO_ERROR
        }),
        "MSG_DONTWAIT" => Some(if is_darwin {
            darwin::MSG_DONTWAIT
        } else {
            linux::MSG_DONTWAIT
        }),
        "MSG_WAITALL" => Some(if is_darwin {
            darwin::MSG_WAITALL
        } else {
            linux::MSG_WAITALL
        }),
        // Linux-only constants
        "SOCK_NONBLOCK" if is_linux => Some(linux::SOCK_NONBLOCK),
        "SOCK_CLOEXEC" if is_linux => Some(linux::SOCK_CLOEXEC),
        "SOL_TCP" if is_linux => Some(linux::SOL_TCP),
        _ => None,
    }
}

/// Unified entry point: cross-platform constants direct, falls back
/// to platform-specific dispatch for divergent names.
pub fn socket_const_value(name: &str, target_os: &str) -> Option<i64> {
    match name {
        // Cross-platform values
        "AF_INET" => Some(AF_INET),
        "SOCK_STREAM" => Some(SOCK_STREAM),
        "SOCK_DGRAM" => Some(SOCK_DGRAM),
        "IPPROTO_TCP" => Some(IPPROTO_TCP),
        "TCP_NODELAY" => Some(TCP_NODELAY),
        "SHUT_RD" => Some(SHUT_RD),
        "SHUT_WR" => Some(SHUT_WR),
        "SHUT_RDWR" => Some(SHUT_RDWR),
        "MSG_PEEK" => Some(MSG_PEEK),
        // Otherwise dispatch on target.
        _ => socket_const_for_target(name, target_os),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-platform socket values stay pinned. Bit-identical on
    /// every supported POSIX target — same value on Linux and Darwin.
    #[test]
    fn cross_platform_socket_constants_pinned() {
        assert_eq!(AF_INET, 2);
        assert_eq!(SOCK_STREAM, 1);
        assert_eq!(SOCK_DGRAM, 2);
        assert_eq!(IPPROTO_TCP, 6);
        assert_eq!(TCP_NODELAY, 1);
        assert_eq!(SHUT_RD, 0);
        assert_eq!(SHUT_WR, 1);
        assert_eq!(SHUT_RDWR, 2);
        assert_eq!(MSG_PEEK, 2);
    }

    /// Linux platform-specific values per
    /// `core/sys/linux/syscall.vr` and `<bits/socket.h>`.
    #[test]
    fn linux_socket_constants_pinned() {
        assert_eq!(linux::AF_INET6, 10);
        assert_eq!(linux::SOL_SOCKET, 1);
        assert_eq!(linux::SO_REUSEADDR, 2);
        assert_eq!(linux::SO_KEEPALIVE, 9);
        assert_eq!(linux::SO_ERROR, 4);
        assert_eq!(linux::MSG_DONTWAIT, 0x40);
        assert_eq!(linux::SOL_TCP, 6);
        // SOCK_NONBLOCK uses octal in libc (= 0o4000 = 2048).
        assert_eq!(linux::SOCK_NONBLOCK, 2048);
    }

    /// Darwin platform-specific values per
    /// `core/sys/darwin/libsystem.vr` and `<sys/socket.h>`.
    #[test]
    fn darwin_socket_constants_pinned() {
        assert_eq!(darwin::AF_INET6, 30);
        assert_eq!(darwin::SOL_SOCKET, 0xFFFF);
        assert_eq!(darwin::SO_REUSEADDR, 0x0004);
        assert_eq!(darwin::SO_KEEPALIVE, 0x0008);
        assert_eq!(darwin::SO_ERROR, 0x1007);
        assert_eq!(darwin::MSG_DONTWAIT, 0x80);
        assert_eq!(darwin::MSG_WAITALL, 0x40);
    }

    /// Linux and Darwin diverge on the documented set — pinning the
    /// disagreement explicitly so a future stdlib reorg can't silently
    /// align values (which would also break runtime ABI).
    #[test]
    fn linux_and_darwin_socket_diverge_where_expected() {
        assert_ne!(linux::AF_INET6, darwin::AF_INET6);
        assert_ne!(linux::SOL_SOCKET, darwin::SOL_SOCKET);
        assert_ne!(linux::SO_REUSEADDR, darwin::SO_REUSEADDR);
        assert_ne!(linux::SO_KEEPALIVE, darwin::SO_KEEPALIVE);
        assert_ne!(linux::SO_ERROR, darwin::SO_ERROR);
        assert_ne!(linux::MSG_DONTWAIT, darwin::MSG_DONTWAIT);
    }

    /// Target dispatch correctly routes per-platform values.
    #[test]
    fn socket_const_for_target_dispatch() {
        assert_eq!(socket_const_for_target("AF_INET6", "linux"), Some(10));
        assert_eq!(socket_const_for_target("AF_INET6", "macos"), Some(30));
        assert_eq!(socket_const_for_target("AF_INET6", "darwin"), Some(30));
        assert_eq!(socket_const_for_target("AF_INET6", "ios"), Some(30));

        assert_eq!(socket_const_for_target("SOL_SOCKET", "linux"), Some(1));
        assert_eq!(
            socket_const_for_target("SOL_SOCKET", "macos"),
            Some(0xFFFF),
        );

        // Cross-platform: returns None.
        assert_eq!(socket_const_for_target("AF_INET", "linux"), None);
        assert_eq!(socket_const_for_target("SOCK_STREAM", "macos"), None);

        // Linux-only constants.
        assert_eq!(
            socket_const_for_target("SOCK_NONBLOCK", "linux"),
            Some(2048),
        );
        // Same name on Darwin: returns None (Darwin doesn't expose it).
        assert_eq!(socket_const_for_target("SOCK_NONBLOCK", "macos"), None);

        // Unknown target.
        assert_eq!(socket_const_for_target("AF_INET6", "windows"), None);
    }

    /// Unified entry point.
    #[test]
    fn socket_const_value_unified_dispatch() {
        // Cross-platform.
        assert_eq!(socket_const_value("AF_INET", "linux"), Some(2));
        assert_eq!(socket_const_value("AF_INET", "macos"), Some(2));
        assert_eq!(socket_const_value("SOCK_STREAM", "linux"), Some(1));
        assert_eq!(socket_const_value("IPPROTO_TCP", "macos"), Some(6));

        // Platform-divergent.
        assert_eq!(socket_const_value("AF_INET6", "linux"), Some(10));
        assert_eq!(socket_const_value("AF_INET6", "macos"), Some(30));
        assert_eq!(socket_const_value("SOL_SOCKET", "linux"), Some(1));
        assert_eq!(socket_const_value("SOL_SOCKET", "macos"), Some(0xFFFF));

        // Unknown name.
        assert_eq!(socket_const_value("NOT_A_SOCKET_CONST", "linux"), None);
    }
}
