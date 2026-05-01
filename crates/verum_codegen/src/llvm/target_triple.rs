//! Cross-compilation-correct TARGET-triple inspection helpers.
//!

//! Codegen decisions that depend on the **target** OS / architecture
//! (syscall numbers, sockaddr layout, socket-option constants, errno
//! function names, …) MUST inspect the LLVM module's target triple,
//! never the compile host's `#[cfg(target_os = "...")]` directives.
//!

//! `#[cfg(target_os = "...")]` binds at *compile-time* of the codegen
//! crate itself — that's the **host** OS. Using it to gate emitted IR
//! silently miscompiles every cross build:
//!

//!  * Build the compiler on x86_64-darwin, target Linux/aarch64 →
//!  host gates compile out the Linux syscall arms, codegen falls
//!  through to libSystem clock_gettime, the resulting Linux binary
//!  references `clock_gettime` from a libc that isn't present /
//!  uses Darwin's CLOCK_MONOTONIC=6 instead of Linux's =1.
//!  * Build on Linux/x86_64, target Darwin/arm64 → host gates compile
//!  IN the Linux syscall arms, codegen emits `syscall` (kernel
//!  trap) into a Darwin binary that crashes on the first call.
//!

//! The LLVM module's *target* triple is the source of truth. These
//! helpers extract OS / architecture flags from that triple at codegen
//! time, so the same compiled `verum_codegen` crate produces correct
//! IR for every target regardless of host.
//!

//! Per user 2026-04-30 directive: "ты уверен что для emit_linux_syscall
//! нужна директива #[cfg(target_os=linux)] - разве это не помешает
//! кросскомпиляции? убедись, что подобного нет в других местах."

use verum_llvm::module::Module;

/// Returns `true` when the LLVM module's target triple denotes Linux.
///

/// Used to select the direct-syscall (libc-free) IR path versus the
/// libSystem / kernel32 paths.
pub fn target_is_linux(module: &Module<'_>) -> bool {
    let triple = module.get_triple();
    triple.as_str().to_string_lossy().contains("linux")
}

/// Returns `true` when the LLVM module's target triple denotes
/// Darwin / Apple platforms (macOS, iOS, tvOS, watchOS — they all
/// share the libSystem ABI and Darwin sockaddr layout).
pub fn target_is_darwin(module: &Module<'_>) -> bool {
    let s = module.get_triple();
    let t = s.as_str().to_string_lossy();
    t.contains("darwin")
        || t.contains("apple")
        || t.contains("macos")
        || t.contains("ios")
        || t.contains("tvos")
        || t.contains("watchos")
}

/// Returns `true` when the LLVM module's target triple denotes Windows.
pub fn target_is_windows(module: &Module<'_>) -> bool {
    let triple = module.get_triple();
    let t = triple.as_str().to_string_lossy();
    t.contains("windows") || t.contains("win32") || t.contains("msvc") || t.contains("mingw")
}

/// Returns `true` when the LLVM module's target triple denotes
/// FreeBSD. FreeBSD differs from Linux in errno, sockaddr layout,
/// and socket option numbers (closer to Darwin's BSD heritage than
/// to Linux).
pub fn target_is_freebsd(module: &Module<'_>) -> bool {
    let triple = module.get_triple();
    triple.as_str().to_string_lossy().contains("freebsd")
}

/// Returns `true` when the LLVM module's target triple denotes
/// **any BSD-derived OS** — FreeBSD, OpenBSD, NetBSD, DragonFlyBSD.
///
/// All BSDs share a common ABI heritage distinct from Linux:
/// - sockaddr layout includes `sin_len` byte (like Darwin)
/// - errno is `__error()` / `__errno_location()`-style not bare global
/// - syscall numbers diverge from Linux's
///
/// Use this when behaviour is identical across all BSD variants;
/// use the OS-specific helpers (`target_is_freebsd`, etc.) when
/// behaviour diverges between them.
pub fn target_is_bsd(module: &Module<'_>) -> bool {
    let triple = module.get_triple();
    let t = triple.as_str().to_string_lossy();
    t.contains("freebsd")
        || t.contains("openbsd")
        || t.contains("netbsd")
        || t.contains("dragonfly")
}

/// Returns `true` when the LLVM module's target triple denotes OpenBSD.
pub fn target_is_openbsd(module: &Module<'_>) -> bool {
    let triple = module.get_triple();
    triple.as_str().to_string_lossy().contains("openbsd")
}

/// Returns `true` when the LLVM module's target triple denotes NetBSD.
pub fn target_is_netbsd(module: &Module<'_>) -> bool {
    let triple = module.get_triple();
    triple.as_str().to_string_lossy().contains("netbsd")
}

/// Returns `true` when the LLVM module's target triple denotes DragonFlyBSD.
pub fn target_is_dragonfly(module: &Module<'_>) -> bool {
    let triple = module.get_triple();
    triple.as_str().to_string_lossy().contains("dragonfly")
}

/// Returns `true` when the LLVM module's target triple denotes
/// aarch64 / arm64.
///

/// Used at syscall-number selection time — most Linux syscall numbers
/// differ between x86_64 and aarch64 (e.g. `SYS_clock_gettime` =
/// 228 on x86_64, 113 on aarch64; `SYS_getpid` = 39 vs 172).
pub fn target_is_aarch64(module: &Module<'_>) -> bool {
    let triple = module.get_triple();
    let t = triple.as_str().to_string_lossy();
    t.contains("aarch64") || t.contains("arm64")
}

/// Returns `true` when the LLVM module's target triple denotes x86_64.
pub fn target_is_x86_64(module: &Module<'_>) -> bool {
    let triple = module.get_triple();
    let t = triple.as_str().to_string_lossy();
    t.contains("x86_64") || t.contains("amd64")
}

// ---------------------------------------------------------------------------
// String-form variants (for callers that have the raw triple string —
// e.g. linker config that runs without an LLVM Module in scope).
// ---------------------------------------------------------------------------

/// `target_is_linux` for callers holding a raw triple string.
///
/// Used by the linker-flag selection in
/// `verum_compiler::pipeline::native_codegen` which decides flags
/// before constructing the LLVM Module.  Same predicate as
/// [`target_is_linux`], just substring-tested directly on the string.
pub fn triple_str_is_linux(triple: &str) -> bool {
    triple.contains("linux")
}

/// `target_is_darwin` for callers holding a raw triple string.
pub fn triple_str_is_darwin(triple: &str) -> bool {
    triple.contains("darwin")
        || triple.contains("apple")
        || triple.contains("macos")
        || triple.contains("ios")
        || triple.contains("tvos")
        || triple.contains("watchos")
}

/// `target_is_windows` for callers holding a raw triple string.
pub fn triple_str_is_windows(triple: &str) -> bool {
    triple.contains("windows")
        || triple.contains("win32")
        || triple.contains("msvc")
        || triple.contains("mingw")
}

/// `target_is_bsd` for callers holding a raw triple string.
/// True for FreeBSD, OpenBSD, NetBSD, DragonFlyBSD.
pub fn triple_str_is_bsd(triple: &str) -> bool {
    triple.contains("freebsd")
        || triple.contains("openbsd")
        || triple.contains("netbsd")
        || triple.contains("dragonfly")
}

/// `target_is_freebsd` for callers holding a raw triple string.
pub fn triple_str_is_freebsd(triple: &str) -> bool {
    triple.contains("freebsd")
}

/// Coarse OS family from a target triple — useful for host-vs-target
/// equality checks without enumerating every variant.
///
/// Returns one of: "darwin", "linux", "windows", "freebsd", "openbsd",
/// "netbsd", "dragonfly", "wasi", "unknown".
pub fn triple_str_os_family(triple: &str) -> &'static str {
    if triple_str_is_darwin(triple) {
        "darwin"
    } else if triple_str_is_linux(triple) {
        "linux"
    } else if triple_str_is_windows(triple) {
        "windows"
    } else if triple.contains("freebsd") {
        "freebsd"
    } else if triple.contains("openbsd") {
        "openbsd"
    } else if triple.contains("netbsd") {
        "netbsd"
    } else if triple.contains("dragonfly") {
        "dragonfly"
    } else if triple.contains("wasi") {
        "wasi"
    } else {
        "unknown"
    }
}

#[cfg(test)]
mod tests {
    // Smoke tests live alongside the module that owns the LLVM
    // Context (we can't construct a Module here without one), so
    // refer to crate::llvm::runtime tests for end-to-end coverage
    // of the dispatch path.
}
