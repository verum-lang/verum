//! Single source of truth for Linux syscall numbers.
//!
//! Linux's no-libc architecture (per
//! `docs/architecture/no-libc-architecture.md`) requires every Verum
//! Linux binary to invoke syscalls directly via the `syscall` (x86_64)
//! / `svc #0` (aarch64) instruction. The numeric mapping from
//! syscall name to `sys_num` differs per architecture — Linux's
//! "generic" syscall ABI (used by aarch64) and the legacy x86_64 ABI
//! assign different numbers to most calls.
//!
//! Authoritative declarations:
//! * x86_64: `<asm/unistd_64.h>` and stdlib `core/sys/linux/syscall.vr`
//! * aarch64: `<asm-generic/unistd.h>` (Linux generic syscall ABI)
//!
//! Pre-this-module, codegen emitter sites repeated the same
//! `if target_is_aarch64(module) { N1 } else { N2 }` pattern with raw
//! integer literals at every syscall emission. `verum_common::linux_syscalls`
//! exposes named constants per architecture + a `syscall_for_arch`
//! dispatch helper.
//!
//! **Architectural rationale**: Linux is a no-libc target (the
//! `verum_codegen` LLVM lowering must NOT emit calls to
//! `libc.so.6`/`libc.musl.so`). Hardcoded syscall numbers are the
//! canonical alternative, but maintaining them requires a single
//! source of truth so that a Linux kernel ABI revision (or the
//! addition of a new `_x` variant) lands in exactly one place.

// ============================================================================
// x86_64 syscall numbers (per `<asm/unistd_64.h>`)
// ============================================================================

/// Linux x86_64 syscall numbers per the legacy x86_64 ABI.
pub mod x86_64 {
    // ---- File I/O ---------------------------------------------------------
    pub const SYS_READ: u64 = 0;
    pub const SYS_WRITE: u64 = 1;
    pub const SYS_OPEN: u64 = 2;
    pub const SYS_CLOSE: u64 = 3;
    pub const SYS_LSEEK: u64 = 8;
    pub const SYS_PREAD64: u64 = 17;
    pub const SYS_PWRITE64: u64 = 18;
    pub const SYS_READV: u64 = 19;
    pub const SYS_WRITEV: u64 = 20;
    pub const SYS_PIPE: u64 = 22;
    pub const SYS_DUP: u64 = 32;
    pub const SYS_DUP2: u64 = 33;
    pub const SYS_OPENAT: u64 = 257;
    pub const SYS_PIPE2: u64 = 293;
    pub const SYS_DUP3: u64 = 292;

    // ---- Memory mapping ---------------------------------------------------
    pub const SYS_MMAP: u64 = 9;
    pub const SYS_MPROTECT: u64 = 10;
    pub const SYS_MUNMAP: u64 = 11;
    pub const SYS_BRK: u64 = 12;
    pub const SYS_MADVISE: u64 = 28;

    // ---- Process / signals ------------------------------------------------
    pub const SYS_RT_SIGACTION: u64 = 13;
    pub const SYS_IOCTL: u64 = 16;
    pub const SYS_SCHED_YIELD: u64 = 24;
    pub const SYS_NANOSLEEP: u64 = 35;
    pub const SYS_GETPID: u64 = 39;
    pub const SYS_CLONE: u64 = 56;
    pub const SYS_FORK: u64 = 57;
    pub const SYS_KILL: u64 = 62;
    pub const SYS_EXIT: u64 = 60;
    pub const SYS_WAIT4: u64 = 61;
    pub const SYS_FCNTL: u64 = 72;
    pub const SYS_GETUID: u64 = 102;
    pub const SYS_GETTID: u64 = 186;
    pub const SYS_FUTEX: u64 = 202;
    pub const SYS_EXIT_GROUP: u64 = 231;
    pub const SYS_GETRANDOM: u64 = 318;

    // ---- Sockets ----------------------------------------------------------
    pub const SYS_SOCKET: u64 = 41;
    pub const SYS_CONNECT: u64 = 42;
    pub const SYS_ACCEPT: u64 = 43;
    pub const SYS_SENDTO: u64 = 44;
    pub const SYS_RECVFROM: u64 = 45;
    pub const SYS_BIND: u64 = 49;
    pub const SYS_LISTEN: u64 = 50;
    pub const SYS_SETSOCKOPT: u64 = 54;
    pub const SYS_GETSOCKOPT: u64 = 55;

    // ---- Time / clocks ----------------------------------------------------
    pub const SYS_CLOCK_GETTIME: u64 = 228;
    pub const SYS_CLOCK_NANOSLEEP: u64 = 230;

    // ---- epoll ------------------------------------------------------------
    pub const SYS_EPOLL_CTL: u64 = 233;
    pub const SYS_EPOLL_PWAIT: u64 = 281;
    pub const SYS_EPOLL_CREATE1: u64 = 291;
}

// ============================================================================
// aarch64 syscall numbers (per `<asm-generic/unistd.h>` — Linux generic ABI)
// ============================================================================

/// Linux aarch64 syscall numbers per the generic syscall ABI.
/// Most newer Linux architectures (aarch64, riscv) share these
/// numbers via `<asm-generic/unistd.h>`.
pub mod aarch64 {
    // ---- File I/O ---------------------------------------------------------
    pub const SYS_READ: u64 = 63;
    pub const SYS_WRITE: u64 = 64;
    pub const SYS_CLOSE: u64 = 57;
    pub const SYS_LSEEK: u64 = 62;
    pub const SYS_PREAD64: u64 = 67;
    pub const SYS_PWRITE64: u64 = 68;
    pub const SYS_READV: u64 = 65;
    pub const SYS_WRITEV: u64 = 66;
    pub const SYS_DUP: u64 = 23;
    pub const SYS_DUP3: u64 = 24;
    pub const SYS_OPENAT: u64 = 56;
    pub const SYS_PIPE2: u64 = 59;

    // ---- Memory mapping ---------------------------------------------------
    pub const SYS_MMAP: u64 = 222;
    pub const SYS_MPROTECT: u64 = 226;
    pub const SYS_MUNMAP: u64 = 215;
    pub const SYS_BRK: u64 = 214;
    pub const SYS_MADVISE: u64 = 233;

    // ---- Process / signals ------------------------------------------------
    pub const SYS_RT_SIGACTION: u64 = 134;
    pub const SYS_IOCTL: u64 = 29;
    pub const SYS_SCHED_YIELD: u64 = 124;
    pub const SYS_NANOSLEEP: u64 = 101;
    pub const SYS_GETPID: u64 = 172;
    pub const SYS_CLONE: u64 = 220;
    pub const SYS_KILL: u64 = 129;
    pub const SYS_EXIT: u64 = 93;
    pub const SYS_WAIT4: u64 = 260;
    pub const SYS_FCNTL: u64 = 25;
    pub const SYS_GETUID: u64 = 174;
    pub const SYS_GETTID: u64 = 178;
    pub const SYS_FUTEX: u64 = 98;
    pub const SYS_EXIT_GROUP: u64 = 94;
    pub const SYS_GETRANDOM: u64 = 278;

    // ---- Sockets ----------------------------------------------------------
    pub const SYS_SOCKET: u64 = 198;
    pub const SYS_CONNECT: u64 = 203;
    pub const SYS_ACCEPT: u64 = 202;
    pub const SYS_SENDTO: u64 = 206;
    pub const SYS_RECVFROM: u64 = 207;
    pub const SYS_BIND: u64 = 200;
    pub const SYS_LISTEN: u64 = 201;
    pub const SYS_SETSOCKOPT: u64 = 208;
    pub const SYS_GETSOCKOPT: u64 = 209;

    // ---- Time / clocks ----------------------------------------------------
    pub const SYS_CLOCK_GETTIME: u64 = 113;
    pub const SYS_CLOCK_NANOSLEEP: u64 = 115;

    // ---- epoll ------------------------------------------------------------
    pub const SYS_EPOLL_CTL: u64 = 21;
    pub const SYS_EPOLL_PWAIT: u64 = 22;
    pub const SYS_EPOLL_CREATE1: u64 = 20;
}

// ============================================================================
// Per-arch dispatch helper
// ============================================================================

/// Resolve a Linux syscall name to its arch-specific number.
/// Returns `None` for unknown names or unknown architectures.
///
/// `target_arch` accepts `"x86_64"` / `"aarch64"` (the Rust target_arch
/// names emitted by `cfg!(target_arch)`). Used by codegen's
/// syscall-emission helpers to replace the ad-hoc
/// `if target_is_aarch64(module) { N1 } else { N2 }` pattern with a
/// named-lookup dispatch.
pub fn syscall_for_arch(name: &str, target_arch: &str) -> Option<u64> {
    match target_arch {
        "x86_64" => syscall_x86_64(name),
        "aarch64" | "arm64" => syscall_aarch64(name),
        _ => None,
    }
}

/// x86_64 lookup helper.
pub fn syscall_x86_64(name: &str) -> Option<u64> {
    match name {
        "SYS_READ" => Some(x86_64::SYS_READ),
        "SYS_WRITE" => Some(x86_64::SYS_WRITE),
        "SYS_OPEN" => Some(x86_64::SYS_OPEN),
        "SYS_CLOSE" => Some(x86_64::SYS_CLOSE),
        "SYS_LSEEK" => Some(x86_64::SYS_LSEEK),
        "SYS_PREAD64" => Some(x86_64::SYS_PREAD64),
        "SYS_PWRITE64" => Some(x86_64::SYS_PWRITE64),
        "SYS_READV" => Some(x86_64::SYS_READV),
        "SYS_WRITEV" => Some(x86_64::SYS_WRITEV),
        "SYS_PIPE" => Some(x86_64::SYS_PIPE),
        "SYS_DUP" => Some(x86_64::SYS_DUP),
        "SYS_DUP2" => Some(x86_64::SYS_DUP2),
        "SYS_OPENAT" => Some(x86_64::SYS_OPENAT),
        "SYS_PIPE2" => Some(x86_64::SYS_PIPE2),
        "SYS_DUP3" => Some(x86_64::SYS_DUP3),
        "SYS_MMAP" => Some(x86_64::SYS_MMAP),
        "SYS_MPROTECT" => Some(x86_64::SYS_MPROTECT),
        "SYS_MUNMAP" => Some(x86_64::SYS_MUNMAP),
        "SYS_BRK" => Some(x86_64::SYS_BRK),
        "SYS_MADVISE" => Some(x86_64::SYS_MADVISE),
        "SYS_RT_SIGACTION" => Some(x86_64::SYS_RT_SIGACTION),
        "SYS_IOCTL" => Some(x86_64::SYS_IOCTL),
        "SYS_SCHED_YIELD" => Some(x86_64::SYS_SCHED_YIELD),
        "SYS_NANOSLEEP" => Some(x86_64::SYS_NANOSLEEP),
        "SYS_GETPID" => Some(x86_64::SYS_GETPID),
        "SYS_CLONE" => Some(x86_64::SYS_CLONE),
        "SYS_FORK" => Some(x86_64::SYS_FORK),
        "SYS_KILL" => Some(x86_64::SYS_KILL),
        "SYS_EXIT" => Some(x86_64::SYS_EXIT),
        "SYS_WAIT4" => Some(x86_64::SYS_WAIT4),
        "SYS_FCNTL" => Some(x86_64::SYS_FCNTL),
        "SYS_GETUID" => Some(x86_64::SYS_GETUID),
        "SYS_GETTID" => Some(x86_64::SYS_GETTID),
        "SYS_FUTEX" => Some(x86_64::SYS_FUTEX),
        "SYS_EXIT_GROUP" => Some(x86_64::SYS_EXIT_GROUP),
        "SYS_GETRANDOM" => Some(x86_64::SYS_GETRANDOM),
        "SYS_SOCKET" => Some(x86_64::SYS_SOCKET),
        "SYS_CONNECT" => Some(x86_64::SYS_CONNECT),
        "SYS_ACCEPT" => Some(x86_64::SYS_ACCEPT),
        "SYS_SENDTO" => Some(x86_64::SYS_SENDTO),
        "SYS_RECVFROM" => Some(x86_64::SYS_RECVFROM),
        "SYS_BIND" => Some(x86_64::SYS_BIND),
        "SYS_LISTEN" => Some(x86_64::SYS_LISTEN),
        "SYS_SETSOCKOPT" => Some(x86_64::SYS_SETSOCKOPT),
        "SYS_GETSOCKOPT" => Some(x86_64::SYS_GETSOCKOPT),
        "SYS_CLOCK_GETTIME" => Some(x86_64::SYS_CLOCK_GETTIME),
        "SYS_CLOCK_NANOSLEEP" => Some(x86_64::SYS_CLOCK_NANOSLEEP),
        "SYS_EPOLL_CTL" => Some(x86_64::SYS_EPOLL_CTL),
        "SYS_EPOLL_PWAIT" => Some(x86_64::SYS_EPOLL_PWAIT),
        "SYS_EPOLL_CREATE1" => Some(x86_64::SYS_EPOLL_CREATE1),
        _ => None,
    }
}

/// aarch64 lookup helper.
pub fn syscall_aarch64(name: &str) -> Option<u64> {
    match name {
        "SYS_READ" => Some(aarch64::SYS_READ),
        "SYS_WRITE" => Some(aarch64::SYS_WRITE),
        "SYS_CLOSE" => Some(aarch64::SYS_CLOSE),
        "SYS_LSEEK" => Some(aarch64::SYS_LSEEK),
        "SYS_PREAD64" => Some(aarch64::SYS_PREAD64),
        "SYS_PWRITE64" => Some(aarch64::SYS_PWRITE64),
        "SYS_READV" => Some(aarch64::SYS_READV),
        "SYS_WRITEV" => Some(aarch64::SYS_WRITEV),
        "SYS_DUP" => Some(aarch64::SYS_DUP),
        "SYS_DUP3" => Some(aarch64::SYS_DUP3),
        "SYS_OPENAT" => Some(aarch64::SYS_OPENAT),
        "SYS_PIPE2" => Some(aarch64::SYS_PIPE2),
        "SYS_MMAP" => Some(aarch64::SYS_MMAP),
        "SYS_MPROTECT" => Some(aarch64::SYS_MPROTECT),
        "SYS_MUNMAP" => Some(aarch64::SYS_MUNMAP),
        "SYS_BRK" => Some(aarch64::SYS_BRK),
        "SYS_MADVISE" => Some(aarch64::SYS_MADVISE),
        "SYS_RT_SIGACTION" => Some(aarch64::SYS_RT_SIGACTION),
        "SYS_IOCTL" => Some(aarch64::SYS_IOCTL),
        "SYS_SCHED_YIELD" => Some(aarch64::SYS_SCHED_YIELD),
        "SYS_NANOSLEEP" => Some(aarch64::SYS_NANOSLEEP),
        "SYS_GETPID" => Some(aarch64::SYS_GETPID),
        "SYS_CLONE" => Some(aarch64::SYS_CLONE),
        "SYS_KILL" => Some(aarch64::SYS_KILL),
        "SYS_EXIT" => Some(aarch64::SYS_EXIT),
        "SYS_WAIT4" => Some(aarch64::SYS_WAIT4),
        "SYS_FCNTL" => Some(aarch64::SYS_FCNTL),
        "SYS_GETUID" => Some(aarch64::SYS_GETUID),
        "SYS_GETTID" => Some(aarch64::SYS_GETTID),
        "SYS_FUTEX" => Some(aarch64::SYS_FUTEX),
        "SYS_EXIT_GROUP" => Some(aarch64::SYS_EXIT_GROUP),
        "SYS_GETRANDOM" => Some(aarch64::SYS_GETRANDOM),
        "SYS_SOCKET" => Some(aarch64::SYS_SOCKET),
        "SYS_CONNECT" => Some(aarch64::SYS_CONNECT),
        "SYS_ACCEPT" => Some(aarch64::SYS_ACCEPT),
        "SYS_SENDTO" => Some(aarch64::SYS_SENDTO),
        "SYS_RECVFROM" => Some(aarch64::SYS_RECVFROM),
        "SYS_BIND" => Some(aarch64::SYS_BIND),
        "SYS_LISTEN" => Some(aarch64::SYS_LISTEN),
        "SYS_SETSOCKOPT" => Some(aarch64::SYS_SETSOCKOPT),
        "SYS_GETSOCKOPT" => Some(aarch64::SYS_GETSOCKOPT),
        "SYS_CLOCK_GETTIME" => Some(aarch64::SYS_CLOCK_GETTIME),
        "SYS_CLOCK_NANOSLEEP" => Some(aarch64::SYS_CLOCK_NANOSLEEP),
        "SYS_EPOLL_CTL" => Some(aarch64::SYS_EPOLL_CTL),
        "SYS_EPOLL_PWAIT" => Some(aarch64::SYS_EPOLL_PWAIT),
        "SYS_EPOLL_CREATE1" => Some(aarch64::SYS_EPOLL_CREATE1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify x86_64 syscall numbers match the canonical Linux x86_64 ABI.
    #[test]
    fn x86_64_syscalls_pinned() {
        // File I/O fundamentals.
        assert_eq!(x86_64::SYS_READ, 0);
        assert_eq!(x86_64::SYS_WRITE, 1);
        assert_eq!(x86_64::SYS_CLOSE, 3);
        // Memory mapping.
        assert_eq!(x86_64::SYS_MMAP, 9);
        assert_eq!(x86_64::SYS_MUNMAP, 11);
        // Process / signals.
        assert_eq!(x86_64::SYS_NANOSLEEP, 35);
        assert_eq!(x86_64::SYS_GETPID, 39);
        assert_eq!(x86_64::SYS_EXIT, 60);
        assert_eq!(x86_64::SYS_GETTID, 186);
        // Time / clocks.
        assert_eq!(x86_64::SYS_CLOCK_GETTIME, 228);
        assert_eq!(x86_64::SYS_EXIT_GROUP, 231);
        // epoll (uniform with Linux 4.5+).
        assert_eq!(x86_64::SYS_EPOLL_CTL, 233);
        assert_eq!(x86_64::SYS_EPOLL_CREATE1, 291);
    }

    /// Verify aarch64 syscall numbers match the Linux generic ABI.
    #[test]
    fn aarch64_syscalls_pinned() {
        // File I/O fundamentals — aarch64 starts at 63 for read/write
        // (vs x86_64's 0/1).
        assert_eq!(aarch64::SYS_READ, 63);
        assert_eq!(aarch64::SYS_WRITE, 64);
        assert_eq!(aarch64::SYS_CLOSE, 57);
        // Memory mapping.
        assert_eq!(aarch64::SYS_MMAP, 222);
        assert_eq!(aarch64::SYS_MUNMAP, 215);
        // Process / signals.
        assert_eq!(aarch64::SYS_NANOSLEEP, 101);
        assert_eq!(aarch64::SYS_GETPID, 172);
        assert_eq!(aarch64::SYS_EXIT, 93);
        assert_eq!(aarch64::SYS_GETTID, 178);
        // Time / clocks.
        assert_eq!(aarch64::SYS_CLOCK_GETTIME, 113);
        assert_eq!(aarch64::SYS_EXIT_GROUP, 94);
        // epoll.
        assert_eq!(aarch64::SYS_EPOLL_CTL, 21);
        assert_eq!(aarch64::SYS_EPOLL_CREATE1, 20);
    }

    /// x86_64 and aarch64 disagree on virtually every syscall number —
    /// pin the disagreement explicitly so a future kernel-ABI revision
    /// can't silently align values.
    #[test]
    fn x86_64_and_aarch64_diverge_where_expected() {
        assert_ne!(x86_64::SYS_READ, aarch64::SYS_READ); // 0 vs 63
        assert_ne!(x86_64::SYS_WRITE, aarch64::SYS_WRITE); // 1 vs 64
        assert_ne!(x86_64::SYS_MMAP, aarch64::SYS_MMAP); // 9 vs 222
        assert_ne!(x86_64::SYS_NANOSLEEP, aarch64::SYS_NANOSLEEP); // 35 vs 101
        assert_ne!(x86_64::SYS_EXIT, aarch64::SYS_EXIT); // 60 vs 93
        assert_ne!(x86_64::SYS_CLOCK_GETTIME, aarch64::SYS_CLOCK_GETTIME); // 228 vs 113
        assert_ne!(x86_64::SYS_GETPID, aarch64::SYS_GETPID); // 39 vs 172
        assert_ne!(x86_64::SYS_EPOLL_CREATE1, aarch64::SYS_EPOLL_CREATE1); // 291 vs 20
    }

    /// Per-arch dispatch resolves canonical names correctly.
    #[test]
    fn syscall_for_arch_dispatch() {
        // x86_64.
        assert_eq!(syscall_for_arch("SYS_READ", "x86_64"), Some(0));
        assert_eq!(syscall_for_arch("SYS_WRITE", "x86_64"), Some(1));
        assert_eq!(syscall_for_arch("SYS_CLOCK_GETTIME", "x86_64"), Some(228));
        assert_eq!(syscall_for_arch("SYS_NANOSLEEP", "x86_64"), Some(35));

        // aarch64 (incl. arm64 alias).
        assert_eq!(syscall_for_arch("SYS_READ", "aarch64"), Some(63));
        assert_eq!(syscall_for_arch("SYS_READ", "arm64"), Some(63));
        assert_eq!(syscall_for_arch("SYS_CLOCK_GETTIME", "aarch64"), Some(113));
        assert_eq!(syscall_for_arch("SYS_NANOSLEEP", "aarch64"), Some(101));

        // Unknown arch.
        assert_eq!(syscall_for_arch("SYS_READ", "x86"), None);
        assert_eq!(syscall_for_arch("SYS_READ", "riscv64"), None);

        // Unknown syscall name.
        assert_eq!(syscall_for_arch("SYS_NOTAREAL", "x86_64"), None);
        assert_eq!(syscall_for_arch("", "aarch64"), None);
    }

    /// x86_64-only / aarch64-only syscalls correctly return None on
    /// the wrong arch (`SYS_OPEN` / `SYS_FORK` / `SYS_PIPE` / `SYS_DUP2`
    /// only exist on x86_64; aarch64 uses `openat`/`clone`/`pipe2`/
    /// `dup3` instead).
    #[test]
    fn arch_specific_syscalls_are_arch_only() {
        // x86_64 has SYS_OPEN, aarch64 doesn't (uses openat).
        assert!(syscall_x86_64("SYS_OPEN").is_some());
        assert!(syscall_aarch64("SYS_OPEN").is_none());
        // x86_64 has SYS_FORK, aarch64 doesn't (uses clone).
        assert!(syscall_x86_64("SYS_FORK").is_some());
        assert!(syscall_aarch64("SYS_FORK").is_none());
        // x86_64 has SYS_DUP2, aarch64 only has SYS_DUP3.
        assert!(syscall_x86_64("SYS_DUP2").is_some());
        assert!(syscall_aarch64("SYS_DUP2").is_none());
    }
}
