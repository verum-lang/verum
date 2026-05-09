//! Single source of truth for platform-library names referenced by
//! the Verum toolchain.
//!
//! Verum's no-libc architecture (per
//! `docs/architecture/no-libc-architecture.md`) restricts the set of
//! dynamic libraries any compiled binary may link against to a tiny
//! per-platform whitelist:
//!
//! | Platform | Permitted libraries                            |
//! |----------|------------------------------------------------|
//! | Linux    | none (direct syscalls via `syscall` / `svc`)   |
//! | macOS    | `libSystem.B.dylib` only (Apple-required)      |
//! | Windows  | `kernel32.dll` + `ntdll.dll` only              |
//! | FreeBSD  | none (direct syscalls)                         |
//!
//! Pre-this-module, the canonical library names were hand-pasted as
//! string literals in 16+ sites across `verum_vbc`, `verum_codegen`,
//! and `verum_compiler` (FFI normalization, runtime emission, archive
//! serialization, linking, profile system, cross-tier loading). A
//! rename (e.g., `libSystem.B.dylib` → a future Apple-mandated
//! variant) would have required a coordinated grep-and-replace across
//! every crate. This module exposes them as named constants — a
//! rename is now one edit.
//!
//! **Drift contract:** any code path that links, dlopens, archives,
//! or string-matches a platform library name MUST consult these
//! constants. Test fixtures and assertion messages may use literals
//! freely; the contract is on production paths only.

// =============================================================================
// macOS — libSystem.B.dylib only
// =============================================================================

/// Apple's umbrella system library on macOS.
///
/// Provides libc-equivalent symbols (`malloc`, `free`, `mach_*`,
/// `pthread_*`, etc.) without bringing in any glibc/musl-style libc.
/// Apple requires every macOS binary to link this library.
///
/// Per CLAUDE.md no-libc invariant: this is the **only** dynamic
/// library a Verum-AOT-compiled macOS binary is allowed to import.
pub const MACOS_LIBSYSTEM: &str = "libSystem.B.dylib";

// =============================================================================
// Windows — kernel32.dll + ntdll.dll only
// =============================================================================

/// Windows core kernel-mode interface library.
///
/// Provides `Heap*`, `VirtualAlloc`, `CreateFile`, `GetSystemInfo`,
/// thread/process primitives, etc. This is one of the two dynamic
/// libraries a Verum-AOT-compiled Windows binary may import; the
/// MSVC CRT (UCRT, vcruntime, etc.) is explicitly excluded.
pub const WINDOWS_KERNEL32: &str = "kernel32.dll";

/// Windows NT native API library.
///
/// Provides lower-level primitives (NTSTATUS-returning
/// `Nt*` / `Rtl*` calls) that codegen targets when a kernel32
/// abstraction would round-trip through extra wrappers. Second of
/// the two permitted Windows libraries.
pub const WINDOWS_NTDLL: &str = "ntdll.dll";

/// Default Windows OS version string used in archive metadata when
/// no specific version is recorded. The 10.0 baseline matches the
/// minimum supported Windows release line.
pub const WINDOWS_DEFAULT_VERSION: &str = "10.0";

// =============================================================================
// Predicate helpers
// =============================================================================

/// Returns true if `name` is one of the canonical macOS-permitted
/// library names. Currently: `libSystem.B.dylib` only.
pub fn is_macos_permitted_library(name: &str) -> bool {
    name == MACOS_LIBSYSTEM
}

/// Returns true if `name` is one of the canonical Windows-permitted
/// library names (`kernel32.dll`, `ntdll.dll`).
pub fn is_windows_permitted_library(name: &str) -> bool {
    matches!(name, WINDOWS_KERNEL32 | WINDOWS_NTDLL)
}

/// Returns true if `name` is any platform-canonical library known
/// to Verum's no-libc architecture (any platform).
///
/// Linux and FreeBSD have no permitted dynamic libraries (direct
/// syscalls), so they don't contribute names.
pub fn is_any_permitted_library(name: &str) -> bool {
    is_macos_permitted_library(name) || is_windows_permitted_library(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drift-protection: the canonical name strings stay pinned.
    /// Any rename must be a deliberate edit reflected here, in
    /// `docs/architecture/no-libc-architecture.md`, and in every
    /// downstream consumer.
    #[test]
    fn canonical_library_names_pinned() {
        assert_eq!(MACOS_LIBSYSTEM, "libSystem.B.dylib");
        assert_eq!(WINDOWS_KERNEL32, "kernel32.dll");
        assert_eq!(WINDOWS_NTDLL, "ntdll.dll");
        assert_eq!(WINDOWS_DEFAULT_VERSION, "10.0");
    }

    /// Per-platform permitted-library predicates correctly classify
    /// canonical names + reject unrelated strings.
    #[test]
    fn platform_predicates() {
        // macOS recognises only libSystem.B.dylib.
        assert!(is_macos_permitted_library(MACOS_LIBSYSTEM));
        assert!(!is_macos_permitted_library(WINDOWS_KERNEL32));
        assert!(!is_macos_permitted_library("libc.so.6"));
        assert!(!is_macos_permitted_library("msvcrt.dll"));

        // Windows recognises kernel32 + ntdll.
        assert!(is_windows_permitted_library(WINDOWS_KERNEL32));
        assert!(is_windows_permitted_library(WINDOWS_NTDLL));
        assert!(!is_windows_permitted_library(MACOS_LIBSYSTEM));
        assert!(!is_windows_permitted_library("ucrt.dll"));

        // Cross-platform recognition.
        assert!(is_any_permitted_library(MACOS_LIBSYSTEM));
        assert!(is_any_permitted_library(WINDOWS_KERNEL32));
        assert!(is_any_permitted_library(WINDOWS_NTDLL));
        assert!(!is_any_permitted_library("libc.so.6"));
    }

    /// Linux and FreeBSD have *no* permitted library names — every
    /// runtime call goes through direct syscalls. Ensure the
    /// predicates correctly return false for typical libc names.
    #[test]
    fn no_libc_invariant_for_unix_targets() {
        for libc_name in [
            "libc.so.6",         // glibc
            "libc.musl.so",      // musl
            "libpthread.so.0",   // pthreads
            "libdl.so.2",        // dlopen
            "ld-linux-x86-64.so.2",
        ] {
            assert!(
                !is_any_permitted_library(libc_name),
                "Linux libc-style name {:?} must NOT be classified as permitted",
                libc_name,
            );
        }
    }
}
