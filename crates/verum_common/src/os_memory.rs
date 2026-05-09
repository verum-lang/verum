//! Single source of truth for OS-level memory-management constants
//! (mmap / madvise on POSIX, VirtualAlloc on Windows).
//!
//! Authoritative declarations live in the Verum stdlib at
//! `core/sys/{linux/mem,darwin/libsystem}.vr` and the Windows-side
//! equivalents (TBD — Windows codegen path is currently embedded in
//! `verum_codegen::llvm::platform_ir`). This module mirrors them as
//! Rust constants that both codegen and the canonical runtime can
//! reference.
//!
//! **Cross-platform** (Linux + Darwin agree):
//! * `PROT_NONE = 0`, `PROT_READ = 1`, `PROT_WRITE = 2`, `PROT_EXEC = 4`
//!   — page-protection bits.
//! * `MAP_SHARED = 1`, `MAP_PRIVATE = 2`, `MAP_FIXED = 0x10` —
//!   mmap mapping mode bits.
//!
//! **Platform-divergent** (Linux vs Darwin):
//!
//! | Constant      | Linux     | Darwin   |
//! |---------------|-----------|----------|
//! | `MAP_ANONYMOUS`| `0x20`   | `0x1000` |
//! | `MAP_ANON`    | `0x20`    | `0x1000` |
//! | `MADV_FREE`   |     8     |     5    |
//! | `MAP_HUGETLB` | `0x40000` | (n/a)    |
//! | `MADV_HUGEPAGE`|    0     | (n/a)    |
//!
//! Pre-this-commit, `verum_vbc::codegen::mod.rs::intrinsic_constant_value`
//! mixed Linux and Darwin values: `MAP_ANONYMOUS = 0x20` (Linux),
//! `MAP_ANON = 0x1000` (Darwin). Same name family with mixed values
//! on the same lookup table — the platform-mismatch bug class fixed
//! for errno / socket / file flags, now applied to memory.
//!
//! **Windows VirtualAlloc / MemoryProtection** values
//! (`MEM_COMMIT`, `MEM_RESERVE`, `MEM_RELEASE`, `PAGE_READWRITE`,
//! `PAGE_NOACCESS`) are not POSIX — they're the Windows-side
//! analogues. Exposed as `windows::*` so codegen targeting Windows
//! reads them through the same dispatch path.

// ============================================================================
// Cross-platform POSIX page-protection bits
// ============================================================================

/// `PROT_NONE` — no access permitted.
pub const PROT_NONE: i64 = 0;
/// `PROT_READ` — page may be read.
pub const PROT_READ: i64 = 1;
/// `PROT_WRITE` — page may be written.
pub const PROT_WRITE: i64 = 2;
/// `PROT_EXEC` — page may be executed.
pub const PROT_EXEC: i64 = 4;

// ============================================================================
// Cross-platform POSIX mmap mapping mode bits
// ============================================================================

/// `MAP_SHARED` — mapping is shared with other processes.
pub const MAP_SHARED: i64 = 1;
/// `MAP_PRIVATE` — copy-on-write private mapping.
pub const MAP_PRIVATE: i64 = 2;
/// `MAP_FIXED` — interpret `addr` exactly (caller-controlled).
pub const MAP_FIXED: i64 = 0x10;

// ============================================================================
// Platform-specific memory constants
// ============================================================================

/// Linux mmap / madvise constants per `<sys/mman.h>` and
/// `core/sys/linux/mem.vr`.
pub mod linux {
    pub const MAP_ANONYMOUS: i64 = 0x20;
    pub const MAP_ANON: i64 = 0x20;
    pub const MAP_NORESERVE: i64 = 0x04000;
    pub const MAP_LOCKED: i64 = 0x02000;
    pub const MAP_POPULATE: i64 = 0x08000;
    pub const MAP_GROWSDOWN: i64 = 0x00100;
    pub const MAP_HUGETLB: i64 = 0x40000;
    pub const MAP_STACK: i64 = 0x20000;

    pub const MADV_NORMAL: i64 = 0;
    pub const MADV_RANDOM: i64 = 1;
    pub const MADV_SEQUENTIAL: i64 = 2;
    pub const MADV_WILLNEED: i64 = 3;
    pub const MADV_DONTNEED: i64 = 4;
    pub const MADV_FREE: i64 = 8;
    pub const MADV_HUGEPAGE: i64 = 14;
    pub const MADV_NOHUGEPAGE: i64 = 15;
}

/// Darwin (macOS / iOS / tvOS) mmap / madvise constants per
/// `<sys/mman.h>` and `core/sys/darwin/libsystem.vr`.
pub mod darwin {
    pub const MAP_ANONYMOUS: i64 = 0x1000;
    pub const MAP_ANON: i64 = 0x1000;
    pub const MAP_NORESERVE: i64 = 0x40;
    pub const MAP_NOCACHE: i64 = 0x400;
    pub const MAP_JIT: i64 = 0x800;

    pub const MADV_NORMAL: i64 = 0;
    pub const MADV_RANDOM: i64 = 1;
    pub const MADV_SEQUENTIAL: i64 = 2;
    pub const MADV_WILLNEED: i64 = 3;
    pub const MADV_DONTNEED: i64 = 4;
    pub const MADV_FREE: i64 = 5;
    /// Darwin extension: hint that pages can be reused without writing back.
    pub const MADV_FREE_REUSABLE: i64 = 7;
    pub const MADV_FREE_REUSE: i64 = 8;
    /// Darwin doesn't have MADV_HUGEPAGE — kept as 0 for codegen
    /// stub-out; downstream code that consults `MADV_HUGEPAGE` on
    /// Darwin should be guarded by `#[cfg(target_os = "linux")]`.
    pub const MADV_HUGEPAGE_STUB: i64 = 0;
}

/// Windows VirtualAlloc / MemoryProtection constants per
/// `<memoryapi.h>`. These are Windows-only — Linux/Darwin paths use
/// mmap / mprotect which take the cross-platform `PROT_*` bits.
pub mod windows {
    /// `VirtualAlloc` allocation type: commit physical storage.
    pub const MEM_COMMIT: i64 = 0x1000;
    /// `VirtualAlloc` allocation type: reserve address-space range.
    pub const MEM_RESERVE: i64 = 0x2000;
    /// `VirtualFree` free type: release entire reserved region.
    pub const MEM_RELEASE: i64 = 0x8000;
    /// `VirtualAlloc` allocation type: enable large pages (huge TLB equivalent).
    pub const MEM_LARGE_PAGES: i64 = 0x20000000;

    /// PAGE_NOACCESS — equivalent to POSIX PROT_NONE.
    pub const PAGE_NOACCESS: i64 = 0x01;
    /// PAGE_READONLY — equivalent to POSIX PROT_READ.
    pub const PAGE_READONLY: i64 = 0x02;
    /// PAGE_READWRITE — equivalent to POSIX PROT_READ | PROT_WRITE.
    pub const PAGE_READWRITE: i64 = 0x04;
    /// PAGE_EXECUTE_READ — equivalent to POSIX PROT_READ | PROT_EXEC.
    pub const PAGE_EXECUTE_READ: i64 = 0x20;
    /// PAGE_EXECUTE_READWRITE — equivalent to PROT_READ|WRITE|EXEC.
    pub const PAGE_EXECUTE_READWRITE: i64 = 0x40;
}

// ============================================================================
// Target-conditional dispatch
// ============================================================================

/// Resolve a memory-management constant by name and target OS.
/// Returns `None` for cross-platform names (use module-level
/// constants), unknown names, or constants unsupported on the target
/// (e.g., `MAP_HUGETLB` on Darwin).
pub fn os_memory_const_for_target(name: &str, target_os: &str) -> Option<i64> {
    let is_darwin = matches!(target_os, "macos" | "darwin" | "ios" | "tvos" | "watchos");
    let is_linux = target_os == "linux";
    let is_windows = target_os == "windows";

    match name {
        // Linux+Darwin divergent
        "MAP_ANONYMOUS" | "MAP_ANON" => {
            if is_darwin {
                Some(darwin::MAP_ANON)
            } else if is_linux {
                Some(linux::MAP_ANON)
            } else {
                None
            }
        }
        "MADV_FREE" => {
            if is_darwin {
                Some(darwin::MADV_FREE)
            } else if is_linux {
                Some(linux::MADV_FREE)
            } else {
                None
            }
        }
        "MADV_NORMAL" => {
            if is_darwin || is_linux {
                Some(0) // Same value on both
            } else {
                None
            }
        }
        "MADV_DONTNEED" => {
            if is_darwin || is_linux {
                Some(4)
            } else {
                None
            }
        }
        // Linux-only mapping flags
        "MAP_HUGETLB" if is_linux => Some(linux::MAP_HUGETLB),
        "MAP_LOCKED" if is_linux => Some(linux::MAP_LOCKED),
        "MAP_NORESERVE" => {
            if is_linux {
                Some(linux::MAP_NORESERVE)
            } else if is_darwin {
                Some(darwin::MAP_NORESERVE)
            } else {
                None
            }
        }
        "MAP_POPULATE" if is_linux => Some(linux::MAP_POPULATE),
        "MAP_STACK" if is_linux => Some(linux::MAP_STACK),
        "MADV_HUGEPAGE" if is_linux => Some(linux::MADV_HUGEPAGE),
        "MADV_NOHUGEPAGE" if is_linux => Some(linux::MADV_NOHUGEPAGE),
        // Windows-only VirtualAlloc constants
        "MEM_COMMIT" if is_windows => Some(windows::MEM_COMMIT),
        "MEM_RESERVE" if is_windows => Some(windows::MEM_RESERVE),
        "MEM_RELEASE" if is_windows => Some(windows::MEM_RELEASE),
        "MEM_LARGE_PAGES" if is_windows => Some(windows::MEM_LARGE_PAGES),
        "PAGE_NOACCESS" if is_windows => Some(windows::PAGE_NOACCESS),
        "PAGE_READONLY" if is_windows => Some(windows::PAGE_READONLY),
        "PAGE_READWRITE" if is_windows => Some(windows::PAGE_READWRITE),
        "PAGE_EXECUTE_READ" if is_windows => Some(windows::PAGE_EXECUTE_READ),
        "PAGE_EXECUTE_READWRITE" if is_windows => Some(windows::PAGE_EXECUTE_READWRITE),
        _ => None,
    }
}

/// Unified entry point: cross-platform direct + platform-specific dispatch.
pub fn os_memory_const_value(name: &str, target_os: &str) -> Option<i64> {
    match name {
        // Cross-platform PROT_*
        "PROT_NONE" => Some(PROT_NONE),
        "PROT_READ" => Some(PROT_READ),
        "PROT_WRITE" => Some(PROT_WRITE),
        "PROT_EXEC" => Some(PROT_EXEC),
        // Cross-platform MAP_* mode bits
        "MAP_SHARED" => Some(MAP_SHARED),
        "MAP_PRIVATE" => Some(MAP_PRIVATE),
        "MAP_FIXED" => Some(MAP_FIXED),
        // Otherwise dispatch on target.
        _ => os_memory_const_for_target(name, target_os),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-platform PROT_* values agree on every supported POSIX
    /// target. PROT_READ = 1, PROT_WRITE = 2, PROT_EXEC = 4 — they're
    /// part of the standard mprotect() contract.
    #[test]
    fn cross_platform_memory_constants_pinned() {
        assert_eq!(PROT_NONE, 0);
        assert_eq!(PROT_READ, 1);
        assert_eq!(PROT_WRITE, 2);
        assert_eq!(PROT_EXEC, 4);
        assert_eq!(MAP_SHARED, 1);
        assert_eq!(MAP_PRIVATE, 2);
        assert_eq!(MAP_FIXED, 0x10);
    }

    /// Linux platform-specific values.
    #[test]
    fn linux_memory_constants_pinned() {
        assert_eq!(linux::MAP_ANONYMOUS, 0x20);
        assert_eq!(linux::MAP_ANON, linux::MAP_ANONYMOUS);
        assert_eq!(linux::MAP_HUGETLB, 0x40000);
        assert_eq!(linux::MADV_FREE, 8);
        assert_eq!(linux::MADV_DONTNEED, 4);
        assert_eq!(linux::MADV_HUGEPAGE, 14);
    }

    /// Darwin platform-specific values.
    #[test]
    fn darwin_memory_constants_pinned() {
        assert_eq!(darwin::MAP_ANONYMOUS, 0x1000);
        assert_eq!(darwin::MAP_ANON, darwin::MAP_ANONYMOUS);
        assert_eq!(darwin::MADV_FREE, 5);
        assert_eq!(darwin::MADV_DONTNEED, 4);
        assert_eq!(darwin::MADV_FREE_REUSABLE, 7);
    }

    /// Windows VirtualAlloc constants.
    #[test]
    fn windows_memory_constants_pinned() {
        assert_eq!(windows::MEM_COMMIT, 0x1000);
        assert_eq!(windows::MEM_RESERVE, 0x2000);
        assert_eq!(windows::MEM_RELEASE, 0x8000);
        assert_eq!(windows::MEM_LARGE_PAGES, 0x20000000);
        assert_eq!(windows::PAGE_NOACCESS, 0x01);
        assert_eq!(windows::PAGE_READWRITE, 0x04);
    }

    /// Linux/Darwin diverge on MAP_ANONYMOUS and MADV_FREE — pin the
    /// disagreement so a future stdlib reorg can't silently align values.
    #[test]
    fn linux_and_darwin_memory_diverge_where_expected() {
        assert_ne!(linux::MAP_ANONYMOUS, darwin::MAP_ANONYMOUS);
        assert_ne!(linux::MAP_ANON, darwin::MAP_ANON);
        assert_ne!(linux::MADV_FREE, darwin::MADV_FREE);
        // MAP_NORESERVE diverges as well.
        assert_ne!(linux::MAP_NORESERVE, darwin::MAP_NORESERVE);
    }

    /// Target dispatch correctly routes per-platform values.
    #[test]
    fn os_memory_const_for_target_dispatch() {
        // Linux/Darwin divergent.
        assert_eq!(os_memory_const_for_target("MAP_ANONYMOUS", "linux"), Some(0x20));
        assert_eq!(
            os_memory_const_for_target("MAP_ANONYMOUS", "macos"),
            Some(0x1000),
        );
        assert_eq!(os_memory_const_for_target("MADV_FREE", "linux"), Some(8));
        assert_eq!(os_memory_const_for_target("MADV_FREE", "darwin"), Some(5));

        // Linux-only.
        assert_eq!(
            os_memory_const_for_target("MAP_HUGETLB", "linux"),
            Some(0x40000),
        );
        assert_eq!(os_memory_const_for_target("MAP_HUGETLB", "macos"), None);
        assert_eq!(os_memory_const_for_target("MADV_HUGEPAGE", "linux"), Some(14));
        assert_eq!(os_memory_const_for_target("MADV_HUGEPAGE", "macos"), None);

        // Windows-only.
        assert_eq!(
            os_memory_const_for_target("MEM_COMMIT", "windows"),
            Some(0x1000),
        );
        assert_eq!(os_memory_const_for_target("MEM_COMMIT", "linux"), None);
        assert_eq!(
            os_memory_const_for_target("PAGE_READWRITE", "windows"),
            Some(4),
        );
        assert_eq!(os_memory_const_for_target("PAGE_READWRITE", "macos"), None);

        // Cross-platform: returns None.
        assert_eq!(os_memory_const_for_target("PROT_READ", "linux"), None);
        assert_eq!(os_memory_const_for_target("MAP_PRIVATE", "macos"), None);
    }

    /// Unified entry point.
    #[test]
    fn os_memory_const_value_unified_dispatch() {
        // Cross-platform.
        assert_eq!(os_memory_const_value("PROT_READ", "linux"), Some(1));
        assert_eq!(os_memory_const_value("PROT_WRITE", "macos"), Some(2));
        assert_eq!(os_memory_const_value("MAP_PRIVATE", "windows"), Some(2));

        // Platform-divergent.
        assert_eq!(os_memory_const_value("MAP_ANONYMOUS", "linux"), Some(0x20));
        assert_eq!(
            os_memory_const_value("MAP_ANONYMOUS", "macos"),
            Some(0x1000),
        );

        // Windows VirtualAlloc.
        assert_eq!(
            os_memory_const_value("MEM_RESERVE", "windows"),
            Some(0x2000),
        );

        // Unknown name.
        assert_eq!(os_memory_const_value("MAP_NOTAFLAG", "linux"), None);
    }
}
