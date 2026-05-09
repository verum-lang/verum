//! Single source of truth for POSIX signal numbers.
//!
//! Authoritative declarations live in the Verum stdlib at
//! `core/sys/darwin/libsystem.vr` (full signal table) and partially
//! in `core/sys/linux/{thread,syscall}.vr`. Linux signals beyond the
//! ones declared in stdlib are sourced from the canonical kernel
//! header `<asm-generic/signal.h>`.
//!
//! Like errno / sockets / file flags, signals split into cross-
//! platform and platform-divergent groups. **Most signals diverge
//! between Linux and Darwin** — only the first 6 numbers
//! (`SIGHUP = 1` through `SIGABRT = 6`) plus a handful at fixed
//! POSIX positions (`SIGFPE = 8`, `SIGKILL = 9`, `SIGSEGV = 11`,
//! `SIGPIPE = 13`, `SIGALRM = 14`, `SIGTERM = 15`) agree. The
//! remaining ~12 signals have different numbers on each platform:
//!
//! | Signal      | Linux | Darwin |
//! |-------------|-------|--------|
//! | `SIGBUS`    |   7   |   10   |
//! | `SIGUSR1`   |   10  |   30   |
//! | `SIGUSR2`   |   12  |   31   |
//! | `SIGCHLD`   |   17  |   20   |
//! | `SIGCONT`   |   18  |   19   |
//! | `SIGSTOP`   |   19  |   17   |
//! | `SIGTSTP`   |   20  |   18   |
//! | `SIGTTIN`   |   21  |   21   (same — historical accident) |
//! | `SIGURG`    |   23  |   16   |
//! | `SIGSYS`    |   31  |   12   |
//!
//! Programs using `kill(pid, SIGUSR1)` compiled on the wrong target
//! send a different signal than expected. This is the most extreme
//! example of the platform-misclassification bug class fixed for
//! errno / sockets / files / mmap.
//!
//! Pre-this-module, `verum_vbc::codegen::mod.rs` had no signal table
//! at all — programs referencing `@const SIGTERM` resolved to
//! whatever `__const_*` lookup found, with no platform awareness.
//! This module establishes the canonical reference even when not yet
//! consumed by the codegen dispatch chain.

// ============================================================================
// Cross-platform POSIX signal numbers
// ============================================================================
//
// These 12 signals have identical numbers on Linux + Darwin (POSIX
// fixed assignments from the original UNIX days).

/// Hangup detected on controlling terminal.
pub const SIGHUP: i64 = 1;
/// Interrupt from keyboard (Ctrl-C).
pub const SIGINT: i64 = 2;
/// Quit from keyboard (Ctrl-\).
pub const SIGQUIT: i64 = 3;
/// Illegal instruction.
pub const SIGILL: i64 = 4;
/// Trace / breakpoint trap.
pub const SIGTRAP: i64 = 5;
/// Abort signal from `abort(3)`.
pub const SIGABRT: i64 = 6;
/// Floating-point exception.
pub const SIGFPE: i64 = 8;
/// Kill (uncatchable, unblockable).
pub const SIGKILL: i64 = 9;
/// Invalid memory reference.
pub const SIGSEGV: i64 = 11;
/// Broken pipe: write to pipe with no readers.
pub const SIGPIPE: i64 = 13;
/// Timer alarm (`alarm(2)` / `setitimer(2)`).
pub const SIGALRM: i64 = 14;
/// Termination signal (default request to terminate).
pub const SIGTERM: i64 = 15;

// ============================================================================
// Platform-specific signal numbers
// ============================================================================

/// Linux signal numbers per `<asm-generic/signal.h>`.
pub mod linux {
    pub const SIGBUS: i64 = 7; // Bus error
    pub const SIGUSR1: i64 = 10; // User-defined signal 1
    pub const SIGUSR2: i64 = 12; // User-defined signal 2
    pub const SIGSTKFLT: i64 = 16; // Stack fault on coprocessor (Linux-only)
    pub const SIGCHLD: i64 = 17; // Child status change
    pub const SIGCONT: i64 = 18; // Continue (after stop)
    pub const SIGSTOP: i64 = 19; // Stop process (uncatchable)
    pub const SIGTSTP: i64 = 20; // Stop typed at terminal
    pub const SIGTTIN: i64 = 21; // Terminal input for background process
    pub const SIGTTOU: i64 = 22; // Terminal output for background process
    pub const SIGURG: i64 = 23; // Urgent condition on socket
    pub const SIGXCPU: i64 = 24; // CPU time limit exceeded
    pub const SIGXFSZ: i64 = 25; // File size limit exceeded
    pub const SIGVTALRM: i64 = 26; // Virtual alarm clock
    pub const SIGPROF: i64 = 27; // Profiling timer expired
    pub const SIGWINCH: i64 = 28; // Window resize
    pub const SIGIO: i64 = 29; // I/O now possible (alias of SIGPOLL)
    pub const SIGPOLL: i64 = SIGIO;
    pub const SIGPWR: i64 = 30; // Power failure (Linux-only)
    pub const SIGSYS: i64 = 31; // Bad system call
}

/// Darwin (macOS / iOS / tvOS) signal numbers per `<sys/signal.h>`
/// and `core/sys/darwin/libsystem.vr`.
pub mod darwin {
    pub const SIGBUS: i64 = 10; // Bus error
    pub const SIGSYS: i64 = 12; // Bad system call
    pub const SIGURG: i64 = 16; // Urgent socket condition
    pub const SIGSTOP: i64 = 17; // Stop (uncatchable)
    pub const SIGTSTP: i64 = 18; // Stop typed at terminal
    pub const SIGCONT: i64 = 19; // Continue
    pub const SIGCHLD: i64 = 20; // Child status change
    pub const SIGTTIN: i64 = 21; // Terminal input for background
    pub const SIGTTOU: i64 = 22; // Terminal output for background
    pub const SIGIO: i64 = 23; // I/O possible
    pub const SIGXCPU: i64 = 24; // CPU time limit
    pub const SIGXFSZ: i64 = 25; // File size limit
    pub const SIGVTALRM: i64 = 26; // Virtual alarm
    pub const SIGPROF: i64 = 27; // Profiling timer
    pub const SIGWINCH: i64 = 28; // Window resize
    pub const SIGINFO: i64 = 29; // Information request (Darwin-only)
    pub const SIGUSR1: i64 = 30; // User-defined signal 1
    pub const SIGUSR2: i64 = 31; // User-defined signal 2
}

// ============================================================================
// Target-conditional dispatch
// ============================================================================

/// Resolve a platform-divergent signal name by name and target OS.
/// Returns `None` for cross-platform names (use module-level
/// constants), unknown names, or unsupported targets.
pub fn signal_for_target(name: &str, target_os: &str) -> Option<i64> {
    let is_darwin = matches!(target_os, "macos" | "darwin" | "ios" | "tvos" | "watchos");
    let is_linux = target_os == "linux";
    if !is_darwin && !is_linux {
        return None;
    }
    match name {
        "SIGBUS" => Some(if is_darwin { darwin::SIGBUS } else { linux::SIGBUS }),
        "SIGUSR1" => Some(if is_darwin {
            darwin::SIGUSR1
        } else {
            linux::SIGUSR1
        }),
        "SIGUSR2" => Some(if is_darwin {
            darwin::SIGUSR2
        } else {
            linux::SIGUSR2
        }),
        "SIGSYS" => Some(if is_darwin { darwin::SIGSYS } else { linux::SIGSYS }),
        "SIGURG" => Some(if is_darwin { darwin::SIGURG } else { linux::SIGURG }),
        "SIGSTOP" => Some(if is_darwin {
            darwin::SIGSTOP
        } else {
            linux::SIGSTOP
        }),
        "SIGTSTP" => Some(if is_darwin {
            darwin::SIGTSTP
        } else {
            linux::SIGTSTP
        }),
        "SIGCONT" => Some(if is_darwin {
            darwin::SIGCONT
        } else {
            linux::SIGCONT
        }),
        "SIGCHLD" => Some(if is_darwin {
            darwin::SIGCHLD
        } else {
            linux::SIGCHLD
        }),
        // Same number on both, but explicitly listed for completeness.
        "SIGTTIN" => Some(21),
        "SIGTTOU" => Some(22),
        // I/O signal: Linux=29, Darwin=23.
        "SIGIO" | "SIGPOLL" => Some(if is_darwin {
            darwin::SIGIO
        } else {
            linux::SIGIO
        }),
        // CPU/file limit signals — same number on both (24/25/26/27/28).
        "SIGXCPU" => Some(24),
        "SIGXFSZ" => Some(25),
        "SIGVTALRM" => Some(26),
        "SIGPROF" => Some(27),
        "SIGWINCH" => Some(28),
        // Linux-only.
        "SIGSTKFLT" if is_linux => Some(linux::SIGSTKFLT),
        "SIGPWR" if is_linux => Some(linux::SIGPWR),
        // Darwin-only.
        "SIGINFO" if is_darwin => Some(darwin::SIGINFO),
        _ => None,
    }
}

/// Unified entry point: cross-platform direct + platform-specific dispatch.
pub fn signal_value(name: &str, target_os: &str) -> Option<i64> {
    match name {
        // Cross-platform (12 signals at fixed POSIX positions).
        "SIGHUP" => Some(SIGHUP),
        "SIGINT" => Some(SIGINT),
        "SIGQUIT" => Some(SIGQUIT),
        "SIGILL" => Some(SIGILL),
        "SIGTRAP" => Some(SIGTRAP),
        "SIGABRT" => Some(SIGABRT),
        "SIGFPE" => Some(SIGFPE),
        "SIGKILL" => Some(SIGKILL),
        "SIGSEGV" => Some(SIGSEGV),
        "SIGPIPE" => Some(SIGPIPE),
        "SIGALRM" => Some(SIGALRM),
        "SIGTERM" => Some(SIGTERM),
        _ => signal_for_target(name, target_os),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-platform signals stay pinned at canonical POSIX numbers.
    #[test]
    fn cross_platform_signals_pinned() {
        assert_eq!(SIGHUP, 1);
        assert_eq!(SIGINT, 2);
        assert_eq!(SIGQUIT, 3);
        assert_eq!(SIGILL, 4);
        assert_eq!(SIGTRAP, 5);
        assert_eq!(SIGABRT, 6);
        assert_eq!(SIGFPE, 8);
        assert_eq!(SIGKILL, 9);
        assert_eq!(SIGSEGV, 11);
        assert_eq!(SIGPIPE, 13);
        assert_eq!(SIGALRM, 14);
        assert_eq!(SIGTERM, 15);
    }

    /// Linux platform-specific signal numbers.
    #[test]
    fn linux_signals_pinned() {
        assert_eq!(linux::SIGBUS, 7);
        assert_eq!(linux::SIGUSR1, 10);
        assert_eq!(linux::SIGUSR2, 12);
        assert_eq!(linux::SIGCHLD, 17);
        assert_eq!(linux::SIGCONT, 18);
        assert_eq!(linux::SIGSTOP, 19);
        assert_eq!(linux::SIGTSTP, 20);
        assert_eq!(linux::SIGURG, 23);
        assert_eq!(linux::SIGSYS, 31);
        assert_eq!(linux::SIGIO, 29);
        assert_eq!(linux::SIGPOLL, linux::SIGIO);
    }

    /// Darwin platform-specific signal numbers.
    #[test]
    fn darwin_signals_pinned() {
        assert_eq!(darwin::SIGBUS, 10);
        assert_eq!(darwin::SIGSYS, 12);
        assert_eq!(darwin::SIGURG, 16);
        assert_eq!(darwin::SIGSTOP, 17);
        assert_eq!(darwin::SIGTSTP, 18);
        assert_eq!(darwin::SIGCONT, 19);
        assert_eq!(darwin::SIGCHLD, 20);
        assert_eq!(darwin::SIGIO, 23);
        assert_eq!(darwin::SIGUSR1, 30);
        assert_eq!(darwin::SIGUSR2, 31);
        assert_eq!(darwin::SIGINFO, 29);
    }

    /// Linux/Darwin diverge on the documented set — pinning the
    /// disagreement explicitly so a future stdlib reorg can't silently
    /// align values (which would break runtime ABI on the affected
    /// platform — `kill(pid, SIGUSR1)` would deliver wrong signal).
    #[test]
    fn linux_and_darwin_signals_diverge_where_expected() {
        assert_ne!(linux::SIGBUS, darwin::SIGBUS);
        assert_ne!(linux::SIGUSR1, darwin::SIGUSR1);
        assert_ne!(linux::SIGUSR2, darwin::SIGUSR2);
        assert_ne!(linux::SIGSYS, darwin::SIGSYS);
        assert_ne!(linux::SIGURG, darwin::SIGURG);
        assert_ne!(linux::SIGCHLD, darwin::SIGCHLD);
        assert_ne!(linux::SIGSTOP, darwin::SIGSTOP);
        assert_ne!(linux::SIGTSTP, darwin::SIGTSTP);
        assert_ne!(linux::SIGCONT, darwin::SIGCONT);
        assert_ne!(linux::SIGIO, darwin::SIGIO);
    }

    /// Target dispatch correctly routes per-platform values.
    #[test]
    fn signal_for_target_dispatch() {
        // Linux/Darwin divergent.
        assert_eq!(signal_for_target("SIGCHLD", "linux"), Some(17));
        assert_eq!(signal_for_target("SIGCHLD", "macos"), Some(20));
        assert_eq!(signal_for_target("SIGUSR1", "linux"), Some(10));
        assert_eq!(signal_for_target("SIGUSR1", "darwin"), Some(30));
        assert_eq!(signal_for_target("SIGBUS", "linux"), Some(7));
        assert_eq!(signal_for_target("SIGBUS", "ios"), Some(10));

        // Linux-only.
        assert_eq!(signal_for_target("SIGPWR", "linux"), Some(30));
        assert_eq!(signal_for_target("SIGPWR", "macos"), None);
        assert_eq!(signal_for_target("SIGSTKFLT", "linux"), Some(16));

        // Darwin-only.
        assert_eq!(signal_for_target("SIGINFO", "macos"), Some(29));
        assert_eq!(signal_for_target("SIGINFO", "linux"), None);

        // Cross-platform: returns None (caller must use module-level
        // const for these — they're target-independent).
        assert_eq!(signal_for_target("SIGTERM", "linux"), None);
        assert_eq!(signal_for_target("SIGKILL", "macos"), None);

        // Unknown target.
        assert_eq!(signal_for_target("SIGCHLD", "windows"), None);
    }

    /// Unified entry point.
    #[test]
    fn signal_value_unified_dispatch() {
        // Cross-platform.
        assert_eq!(signal_value("SIGTERM", "linux"), Some(15));
        assert_eq!(signal_value("SIGTERM", "macos"), Some(15));
        assert_eq!(signal_value("SIGKILL", "ios"), Some(9));

        // Platform-divergent.
        assert_eq!(signal_value("SIGCHLD", "linux"), Some(17));
        assert_eq!(signal_value("SIGCHLD", "macos"), Some(20));

        // Same value on both platforms despite different declaration paths.
        assert_eq!(signal_value("SIGTTIN", "linux"), Some(21));
        assert_eq!(signal_value("SIGTTIN", "darwin"), Some(21));

        // Unknown name.
        assert_eq!(signal_value("SIGNOTAREALSIGNAL", "linux"), None);
    }
}
