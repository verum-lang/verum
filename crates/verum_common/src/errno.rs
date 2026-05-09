//! Single source of truth for POSIX `errno` constants.
//!
//! Authoritative declarations live in the Verum stdlib at
//! `core/sys/darwin/errno.vr` and `core/sys/linux/errno.vr`. Both
//! platform-specific files declare the same set of error names but
//! with values diverging across platforms — Linux and Darwin use
//! different numeric assignments for several errors that are common
//! sources of cross-platform bugs:
//!
//! | Errno          | Linux | Darwin |
//! |----------------|-------|--------|
//! | `EAGAIN`       |   11  |   35   |
//! | `EWOULDBLOCK`  |   11  |   35   |
//! | `EADDRINUSE`   |   98  |   48   |
//! | `EADDRNOTAVAIL`|   99  |   49   |
//! | `ENETUNREACH`  |  101  |   51   |
//! | `ECONNABORTED` |  103  |   53   |
//! | `ECONNRESET`   |  104  |   54   |
//! | `ENOTCONN`     |  107  |   57   |
//! | `ETIMEDOUT`    |  110  |   60   |
//! | `ECONNREFUSED` |  111  |   61   |
//! | `EALREADY`     |  114  |   37   |
//! | `EINPROGRESS`  |  115  |   36   |
//!
//! Pre-this-module, the codegen `intrinsic_constant_value` table
//! consumed by `@const EPERM` / `@const EAGAIN` style references
//! mixed values from both platforms (e.g., `EAGAIN = 11` (Linux) but
//! `EWOULDBLOCK = 35` (Darwin)) — a real platform-misclassification
//! bug masked by the single non-conditional table. This module
//! exposes both platforms' values as named submodules; callers
//! dispatch on the build target.
//!
//! Common errno values that agree across all POSIX platforms
//! (`EPERM = 1`, `ENOENT = 2`, etc.) live at module level for
//! convenience.
//!
//! Drift contract: each submodule's tests assert the values match
//! the canonical Verum stdlib `core/sys/{darwin,linux}/errno.vr`
//! file (mirrored manually because the stdlib parser is
//! downstream of `verum_common`).

// ============================================================================
// Cross-platform POSIX errno (values agreed on Linux + Darwin)
// ============================================================================
//
// These 22 errno values are bit-for-bit identical on Linux and
// Darwin (and on most other POSIX systems). Codegen and runtime
// can reference these unconditionally without worrying about the
// build target.

/// Operation not permitted.
pub const EPERM: i64 = 1;
/// No such file or directory.
pub const ENOENT: i64 = 2;
/// No such process.
pub const ESRCH: i64 = 3;
/// Interrupted system call.
pub const EINTR: i64 = 4;
/// I/O error.
pub const EIO: i64 = 5;
/// No such device or address.
pub const ENXIO: i64 = 6;
/// Argument list too long.
pub const E2BIG: i64 = 7;
/// Exec format error.
pub const ENOEXEC: i64 = 8;
/// Bad file descriptor.
pub const EBADF: i64 = 9;
/// No child processes.
pub const ECHILD: i64 = 10;
/// Out of memory.
pub const ENOMEM: i64 = 12;
/// Permission denied.
pub const EACCES: i64 = 13;
/// Bad address.
pub const EFAULT: i64 = 14;
/// Device or resource busy.
pub const EBUSY: i64 = 16;
/// File exists.
pub const EEXIST: i64 = 17;
/// No such device.
pub const ENODEV: i64 = 19;
/// Not a directory.
pub const ENOTDIR: i64 = 20;
/// Is a directory.
pub const EISDIR: i64 = 21;
/// Invalid argument.
pub const EINVAL: i64 = 22;
/// Too many open files.
pub const EMFILE: i64 = 24;
/// No space left on device.
pub const ENOSPC: i64 = 28;
/// Broken pipe.
pub const EPIPE: i64 = 32;
/// Numerical result out of range.
pub const ERANGE: i64 = 34;

// ============================================================================
// Platform-specific errno (Linux / Darwin diverge)
// ============================================================================

/// Linux errno values (per Linux kernel `<asm-generic/errno.h>` and
/// `core/sys/linux/errno.vr`).
pub mod linux {
    pub const EAGAIN: i64 = 11;
    pub const EWOULDBLOCK: i64 = 11;
    pub const ENOTEMPTY: i64 = 39;
    pub const ENOSYS: i64 = 38;
    pub const EADDRINUSE: i64 = 98;
    pub const EADDRNOTAVAIL: i64 = 99;
    pub const ENETUNREACH: i64 = 101;
    pub const ECONNABORTED: i64 = 103;
    pub const ECONNRESET: i64 = 104;
    pub const ENOTCONN: i64 = 107;
    pub const ETIMEDOUT: i64 = 110;
    pub const ECONNREFUSED: i64 = 111;
    pub const EALREADY: i64 = 114;
    pub const EINPROGRESS: i64 = 115;
}

/// Darwin (macOS / iOS / tvOS) errno values (per `<sys/errno.h>` and
/// `core/sys/darwin/errno.vr`).
pub mod darwin {
    pub const EAGAIN: i64 = 35;
    pub const EWOULDBLOCK: i64 = 35;
    pub const EINPROGRESS: i64 = 36;
    pub const EALREADY: i64 = 37;
    pub const EADDRINUSE: i64 = 48;
    pub const EADDRNOTAVAIL: i64 = 49;
    pub const ENETUNREACH: i64 = 51;
    pub const ECONNABORTED: i64 = 53;
    pub const ECONNRESET: i64 = 54;
    pub const ENOTCONN: i64 = 57;
    pub const ETIMEDOUT: i64 = 60;
    pub const ECONNREFUSED: i64 = 61;
    pub const ENOTEMPTY: i64 = 66;
    pub const ENOSYS: i64 = 78;
}

// ============================================================================
// Target-conditional dispatch
// ============================================================================

/// Resolve a platform-divergent errno value by canonical name and
/// target-OS string (the build target — `"linux"` / `"macos"` /
/// `"darwin"` / `"ios"`). Returns `None` for cross-platform errnos
/// (use the module-level constants for those) or unknown names.
///
/// Used by codegen when emitting target-specific `@const ETIMEDOUT`-
/// style references — the build target dictates the value.
pub fn errno_for_target(name: &str, target_os: &str) -> Option<i64> {
    let is_darwin = matches!(target_os, "macos" | "darwin" | "ios" | "tvos" | "watchos");
    let is_linux = target_os == "linux";
    if !is_darwin && !is_linux {
        return None;
    }
    match name {
        "EAGAIN" | "EWOULDBLOCK" => {
            Some(if is_darwin { darwin::EAGAIN } else { linux::EAGAIN })
        }
        "EINPROGRESS" => Some(if is_darwin {
            darwin::EINPROGRESS
        } else {
            linux::EINPROGRESS
        }),
        "EALREADY" => Some(if is_darwin {
            darwin::EALREADY
        } else {
            linux::EALREADY
        }),
        "EADDRINUSE" => Some(if is_darwin {
            darwin::EADDRINUSE
        } else {
            linux::EADDRINUSE
        }),
        "EADDRNOTAVAIL" => Some(if is_darwin {
            darwin::EADDRNOTAVAIL
        } else {
            linux::EADDRNOTAVAIL
        }),
        "ENETUNREACH" => Some(if is_darwin {
            darwin::ENETUNREACH
        } else {
            linux::ENETUNREACH
        }),
        "ECONNABORTED" => Some(if is_darwin {
            darwin::ECONNABORTED
        } else {
            linux::ECONNABORTED
        }),
        "ECONNRESET" => Some(if is_darwin {
            darwin::ECONNRESET
        } else {
            linux::ECONNRESET
        }),
        "ENOTCONN" => Some(if is_darwin {
            darwin::ENOTCONN
        } else {
            linux::ENOTCONN
        }),
        "ETIMEDOUT" => Some(if is_darwin {
            darwin::ETIMEDOUT
        } else {
            linux::ETIMEDOUT
        }),
        "ECONNREFUSED" => Some(if is_darwin {
            darwin::ECONNREFUSED
        } else {
            linux::ECONNREFUSED
        }),
        "ENOTEMPTY" => Some(if is_darwin {
            darwin::ENOTEMPTY
        } else {
            linux::ENOTEMPTY
        }),
        "ENOSYS" => Some(if is_darwin {
            darwin::ENOSYS
        } else {
            linux::ENOSYS
        }),
        _ => None,
    }
}

/// Resolve any errno (cross-platform or platform-divergent) by name
/// for the given build target. Returns the cross-platform constant
/// directly when defined; falls back to `errno_for_target` for
/// platform-divergent names. Returns `None` for unknown names.
pub fn errno_value(name: &str, target_os: &str) -> Option<i64> {
    match name {
        // Cross-platform values (agreed on all POSIX systems).
        "EPERM" => Some(EPERM),
        "ENOENT" => Some(ENOENT),
        "ESRCH" => Some(ESRCH),
        "EINTR" => Some(EINTR),
        "EIO" => Some(EIO),
        "ENXIO" => Some(ENXIO),
        "E2BIG" => Some(E2BIG),
        "ENOEXEC" => Some(ENOEXEC),
        "EBADF" => Some(EBADF),
        "ECHILD" => Some(ECHILD),
        "ENOMEM" => Some(ENOMEM),
        "EACCES" => Some(EACCES),
        "EFAULT" => Some(EFAULT),
        "EBUSY" => Some(EBUSY),
        "EEXIST" => Some(EEXIST),
        "ENODEV" => Some(ENODEV),
        "ENOTDIR" => Some(ENOTDIR),
        "EISDIR" => Some(EISDIR),
        "EINVAL" => Some(EINVAL),
        "EMFILE" => Some(EMFILE),
        "ENOSPC" => Some(ENOSPC),
        "EPIPE" => Some(EPIPE),
        "ERANGE" => Some(ERANGE),
        // Otherwise dispatch on target.
        _ => errno_for_target(name, target_os),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-platform errno values stay pinned. These are the standard
    /// POSIX assignments — bit-identical on every supported target.
    #[test]
    fn cross_platform_errno_pinned() {
        assert_eq!(EPERM, 1);
        assert_eq!(ENOENT, 2);
        assert_eq!(ESRCH, 3);
        assert_eq!(EINTR, 4);
        assert_eq!(EIO, 5);
        assert_eq!(EBADF, 9);
        assert_eq!(ENOMEM, 12);
        assert_eq!(EACCES, 13);
        assert_eq!(EINVAL, 22);
        assert_eq!(EPIPE, 32);
        assert_eq!(ERANGE, 34);
    }

    /// Linux platform-specific values (per `core/sys/linux/errno.vr`).
    #[test]
    fn linux_errno_pinned() {
        assert_eq!(linux::EAGAIN, 11);
        assert_eq!(linux::EWOULDBLOCK, 11);
        assert_eq!(linux::EAGAIN, linux::EWOULDBLOCK, "POSIX requires same value");
        assert_eq!(linux::EADDRINUSE, 98);
        assert_eq!(linux::ECONNREFUSED, 111);
        assert_eq!(linux::ETIMEDOUT, 110);
        assert_eq!(linux::EINPROGRESS, 115);
    }

    /// Darwin platform-specific values (per `core/sys/darwin/errno.vr`).
    #[test]
    fn darwin_errno_pinned() {
        assert_eq!(darwin::EAGAIN, 35);
        assert_eq!(darwin::EWOULDBLOCK, 35);
        assert_eq!(darwin::EAGAIN, darwin::EWOULDBLOCK, "POSIX requires same value");
        assert_eq!(darwin::EADDRINUSE, 48);
        assert_eq!(darwin::ECONNREFUSED, 61);
        assert_eq!(darwin::ETIMEDOUT, 60);
        assert_eq!(darwin::EINPROGRESS, 36);
    }

    /// Linux and Darwin disagree on the platform-divergent set —
    /// pin the disagreement explicitly so a future stdlib reorg
    /// can't silently align them (which would also break ABI on
    /// the runtime side).
    #[test]
    fn linux_and_darwin_diverge_where_expected() {
        assert_ne!(linux::EAGAIN, darwin::EAGAIN);
        assert_ne!(linux::EADDRINUSE, darwin::EADDRINUSE);
        assert_ne!(linux::ETIMEDOUT, darwin::ETIMEDOUT);
        assert_ne!(linux::ECONNREFUSED, darwin::ECONNREFUSED);
        assert_ne!(linux::EINPROGRESS, darwin::EINPROGRESS);
        assert_ne!(linux::EALREADY, darwin::EALREADY);
    }

    /// `errno_for_target` correctly dispatches to per-platform values.
    #[test]
    fn errno_for_target_dispatch() {
        assert_eq!(errno_for_target("EAGAIN", "linux"), Some(11));
        assert_eq!(errno_for_target("EAGAIN", "macos"), Some(35));
        assert_eq!(errno_for_target("EAGAIN", "darwin"), Some(35));
        assert_eq!(errno_for_target("EAGAIN", "ios"), Some(35));

        assert_eq!(errno_for_target("ETIMEDOUT", "linux"), Some(110));
        assert_eq!(errno_for_target("ETIMEDOUT", "macos"), Some(60));

        // Cross-platform errnos return None — caller should use the
        // module-level constant directly.
        assert_eq!(errno_for_target("EPERM", "linux"), None);
        assert_eq!(errno_for_target("EBADF", "macos"), None);

        // Unknown target returns None.
        assert_eq!(errno_for_target("EAGAIN", "windows"), None);
    }

    /// `errno_value` is the unified entry point: returns cross-platform
    /// constants directly, falls back to platform-specific dispatch.
    #[test]
    fn errno_value_unified_dispatch() {
        // Cross-platform.
        assert_eq!(errno_value("EPERM", "linux"), Some(1));
        assert_eq!(errno_value("EPERM", "macos"), Some(1));
        assert_eq!(errno_value("EBADF", "linux"), Some(9));
        assert_eq!(errno_value("EBADF", "macos"), Some(9));

        // Platform-divergent.
        assert_eq!(errno_value("EAGAIN", "linux"), Some(11));
        assert_eq!(errno_value("EAGAIN", "macos"), Some(35));
        assert_eq!(errno_value("ETIMEDOUT", "linux"), Some(110));
        assert_eq!(errno_value("ETIMEDOUT", "macos"), Some(60));

        // Unknown name.
        assert_eq!(errno_value("ENOTAREALERROR", "linux"), None);
    }
}
