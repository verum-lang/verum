//! Single source of truth for POSIX file-system constants.
//!
//! Authoritative declarations live in the Verum stdlib at
//! `core/sys/darwin/libsystem.vr` and `core/sys/linux/syscall.vr`.
//! Like POSIX errno and socket constants, file-system flags split into
//! cross-platform and platform-divergent groups.
//!
//! **Cross-platform** (Linux + Darwin agree):
//! * Access modes: `O_RDONLY = 0`, `O_WRONLY = 1`, `O_RDWR = 2`
//! * Seek whences: `SEEK_SET = 0`, `SEEK_CUR = 1`, `SEEK_END = 2`
//!
//! **Platform-divergent**:
//!
//! | Flag           | Linux         | Darwin        |
//! |----------------|---------------|---------------|
//! | `O_NONBLOCK`   | `0o4000`=2048 | `0x0004`=4    |
//! | `O_APPEND`     | `0o2000`=1024 | `0x0008`=8    |
//! | `O_CREAT`      | `0o100`=64    | `0x0200`=512  |
//! | `O_TRUNC`      | `0o1000`=512  | `0x0400`=1024 |
//! | `O_CLOEXEC`    | `0o2000000`   | `0x1000000`   |
//! | `O_DIRECTORY`  | `0o200000`    | `0x100000`    |
//! | `O_NOFOLLOW`   | `0o400000`    | `0x100`       |
//!
//! Pre-this-module, `verum_vbc::codegen::mod.rs::intrinsic_constant_value`
//! hardcoded the *Darwin* values for every divergent flag — programs
//! compiled to Linux receive wrong `open(2)` flags, leading to silent
//! ENOENT/EINVAL failures or incorrect file-mode semantics. This is
//! the same platform-misclassification bug class fixed for errno
//! (commit 3ce48ddd8) and socket constants (commit 40fcf74f2).

// ============================================================================
// Cross-platform POSIX file-system constants
// ============================================================================

/// Access mode: read-only.
pub const O_RDONLY: i64 = 0;
/// Access mode: write-only.
pub const O_WRONLY: i64 = 1;
/// Access mode: read+write.
pub const O_RDWR: i64 = 2;

/// Seek whence: set offset to absolute position.
pub const SEEK_SET: i64 = 0;
/// Seek whence: set offset relative to current position.
pub const SEEK_CUR: i64 = 1;
/// Seek whence: set offset relative to end of file.
pub const SEEK_END: i64 = 2;

// ============================================================================
// Platform-specific file-open flags
// ============================================================================

/// Linux file-open flag values (per `<bits/fcntl-linux.h>` and
/// `core/sys/linux/syscall.vr`). Linux uses octal literals
/// historically — values shown in decimal for clarity.
pub mod linux {
    pub const O_NONBLOCK: i64 = 0o4000; // 2048
    pub const O_APPEND: i64 = 0o2000; // 1024
    pub const O_CREAT: i64 = 0o100; // 64
    pub const O_TRUNC: i64 = 0o1000; // 512
    pub const O_CLOEXEC: i64 = 0o2000000; // 1048576
    pub const O_DIRECTORY: i64 = 0o200000; // 65536
    pub const O_NOFOLLOW: i64 = 0o400000; // 131072
    pub const O_DSYNC: i64 = 0o10000; // 4096
    pub const O_SYNC: i64 = 0o4010000; // 1052672
    pub const O_EXCL: i64 = 0o200; // 128
}

/// Darwin (macOS / iOS / tvOS) file-open flag values (per
/// `<sys/fcntl.h>` and `core/sys/darwin/libsystem.vr`). Darwin uses
/// hex literals historically.
pub mod darwin {
    pub const O_NONBLOCK: i64 = 0x0004;
    pub const O_APPEND: i64 = 0x0008;
    pub const O_CREAT: i64 = 0x0200;
    pub const O_TRUNC: i64 = 0x0400;
    pub const O_CLOEXEC: i64 = 0x1000000;
    pub const O_DIRECTORY: i64 = 0x100000;
    pub const O_NOFOLLOW: i64 = 0x100;
    pub const O_DSYNC: i64 = 0x400000;
    pub const O_SYNC: i64 = 0x80;
    pub const O_EXCL: i64 = 0x800;
}

// ============================================================================
// Target-conditional dispatch
// ============================================================================

/// Resolve a platform-divergent file-open flag by name and target OS.
/// Returns `None` for cross-platform names (use module-level constants),
/// unknown names, or unknown targets.
pub fn file_flag_for_target(name: &str, target_os: &str) -> Option<i64> {
    let is_darwin = matches!(target_os, "macos" | "darwin" | "ios" | "tvos" | "watchos");
    let is_linux = target_os == "linux";
    if !is_darwin && !is_linux {
        return None;
    }
    match name {
        "O_NONBLOCK" => Some(if is_darwin {
            darwin::O_NONBLOCK
        } else {
            linux::O_NONBLOCK
        }),
        "O_APPEND" => Some(if is_darwin {
            darwin::O_APPEND
        } else {
            linux::O_APPEND
        }),
        "O_CREAT" => Some(if is_darwin {
            darwin::O_CREAT
        } else {
            linux::O_CREAT
        }),
        "O_TRUNC" => Some(if is_darwin {
            darwin::O_TRUNC
        } else {
            linux::O_TRUNC
        }),
        "O_CLOEXEC" => Some(if is_darwin {
            darwin::O_CLOEXEC
        } else {
            linux::O_CLOEXEC
        }),
        "O_DIRECTORY" => Some(if is_darwin {
            darwin::O_DIRECTORY
        } else {
            linux::O_DIRECTORY
        }),
        "O_NOFOLLOW" => Some(if is_darwin {
            darwin::O_NOFOLLOW
        } else {
            linux::O_NOFOLLOW
        }),
        "O_DSYNC" => Some(if is_darwin {
            darwin::O_DSYNC
        } else {
            linux::O_DSYNC
        }),
        "O_SYNC" => Some(if is_darwin {
            darwin::O_SYNC
        } else {
            linux::O_SYNC
        }),
        "O_EXCL" => Some(if is_darwin {
            darwin::O_EXCL
        } else {
            linux::O_EXCL
        }),
        _ => None,
    }
}

/// Unified entry point: cross-platform constants direct, falls back
/// to platform-specific dispatch for divergent names.
pub fn file_flag_value(name: &str, target_os: &str) -> Option<i64> {
    match name {
        // Cross-platform values
        "O_RDONLY" => Some(O_RDONLY),
        "O_WRONLY" => Some(O_WRONLY),
        "O_RDWR" => Some(O_RDWR),
        "SEEK_SET" => Some(SEEK_SET),
        "SEEK_CUR" => Some(SEEK_CUR),
        "SEEK_END" => Some(SEEK_END),
        // Otherwise dispatch on target.
        _ => file_flag_for_target(name, target_os),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-platform values stay pinned. Bit-identical on every POSIX target.
    #[test]
    fn cross_platform_file_constants_pinned() {
        assert_eq!(O_RDONLY, 0);
        assert_eq!(O_WRONLY, 1);
        assert_eq!(O_RDWR, 2);
        assert_eq!(SEEK_SET, 0);
        assert_eq!(SEEK_CUR, 1);
        assert_eq!(SEEK_END, 2);
    }

    /// Linux platform-specific values per `<bits/fcntl-linux.h>` and
    /// `core/sys/linux/syscall.vr`.
    #[test]
    fn linux_file_constants_pinned() {
        assert_eq!(linux::O_NONBLOCK, 2048);
        assert_eq!(linux::O_APPEND, 1024);
        assert_eq!(linux::O_CREAT, 64);
        assert_eq!(linux::O_TRUNC, 512);
        assert_eq!(linux::O_CLOEXEC, 0o2000000);
        assert_eq!(linux::O_DIRECTORY, 0o200000);
        assert_eq!(linux::O_EXCL, 128);
    }

    /// Darwin platform-specific values per `<sys/fcntl.h>` and
    /// `core/sys/darwin/libsystem.vr`.
    #[test]
    fn darwin_file_constants_pinned() {
        assert_eq!(darwin::O_NONBLOCK, 4);
        assert_eq!(darwin::O_APPEND, 8);
        assert_eq!(darwin::O_CREAT, 0x200);
        assert_eq!(darwin::O_TRUNC, 0x400);
        assert_eq!(darwin::O_CLOEXEC, 0x1000000);
        assert_eq!(darwin::O_DIRECTORY, 0x100000);
        assert_eq!(darwin::O_NOFOLLOW, 0x100);
        assert_eq!(darwin::O_EXCL, 0x800);
    }

    /// Linux and Darwin diverge — pin the disagreement so a future
    /// stdlib reorg can't silently align values (which would also
    /// break the runtime on the affected platform).
    #[test]
    fn linux_and_darwin_files_diverge_where_expected() {
        assert_ne!(linux::O_NONBLOCK, darwin::O_NONBLOCK);
        assert_ne!(linux::O_APPEND, darwin::O_APPEND);
        assert_ne!(linux::O_CREAT, darwin::O_CREAT);
        assert_ne!(linux::O_TRUNC, darwin::O_TRUNC);
        assert_ne!(linux::O_CLOEXEC, darwin::O_CLOEXEC);
        assert_ne!(linux::O_DIRECTORY, darwin::O_DIRECTORY);
        assert_ne!(linux::O_NOFOLLOW, darwin::O_NOFOLLOW);
    }

    /// Target dispatch correctly routes per-platform values.
    #[test]
    fn file_flag_for_target_dispatch() {
        // Platform-divergent.
        assert_eq!(file_flag_for_target("O_CREAT", "linux"), Some(64));
        assert_eq!(file_flag_for_target("O_CREAT", "macos"), Some(0x200));
        assert_eq!(file_flag_for_target("O_TRUNC", "linux"), Some(512));
        assert_eq!(file_flag_for_target("O_TRUNC", "macos"), Some(0x400));
        assert_eq!(file_flag_for_target("O_NONBLOCK", "linux"), Some(2048));
        assert_eq!(file_flag_for_target("O_NONBLOCK", "darwin"), Some(4));

        // Cross-platform: returns None.
        assert_eq!(file_flag_for_target("O_RDONLY", "linux"), None);
        assert_eq!(file_flag_for_target("SEEK_SET", "macos"), None);

        // Unknown target.
        assert_eq!(file_flag_for_target("O_CREAT", "windows"), None);
    }

    /// Unified entry point.
    #[test]
    fn file_flag_value_unified_dispatch() {
        // Cross-platform.
        assert_eq!(file_flag_value("O_RDONLY", "linux"), Some(0));
        assert_eq!(file_flag_value("O_WRONLY", "macos"), Some(1));
        assert_eq!(file_flag_value("SEEK_END", "linux"), Some(2));

        // Platform-divergent.
        assert_eq!(file_flag_value("O_CREAT", "linux"), Some(64));
        assert_eq!(file_flag_value("O_CREAT", "macos"), Some(0x200));
        assert_eq!(file_flag_value("O_CLOEXEC", "linux"), Some(0o2000000));
        assert_eq!(file_flag_value("O_CLOEXEC", "macos"), Some(0x1000000));

        // Unknown name.
        assert_eq!(file_flag_value("O_NOTAFLAG", "linux"), None);
    }
}
