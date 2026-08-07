//! Platform entropy — the interpreter's only source of secure randomness.
//!
//! # Two contracts over one implementation
//!
//! [`try_fill_secure`] is the draw: it reports what the platform said.
//! [`fill_secure`] adds the contract the LANGUAGE needs — a request for
//! secure randomness either succeeds or the process dies. There is
//! deliberately no PRNG fallback: a caller asks for secure randomness
//! because a key, nonce, salt or session token depends on it, and a
//! guessable answer is worse than no answer — it fails silently,
//! passes every test, and is found by an attacker rather than by us.
//!
//! The raw-syscall shim (`SysGetentropy`) keeps the reporting contract,
//! because a shim's job is to relay the kernel's answer. Nothing that
//! needs secrecy may use it without checking.
//!
//! # What this replaces
//!
//! Three separate entropy paths existed in the dispatch handlers, and
//! the two used by `core.random.secure` had these defects:
//!
//!  * **The return code was discarded.** `getentropy(buf, 8)` and
//!    `syscall(SYS_getrandom, buf, 8, 0)` were issued and ignored. On
//!    failure the stack buffer kept its initial `[0u8; 8]` and the
//!    function returned zero — all-zero key material, reported as
//!    success.
//!  * **Non-Linux, non-macOS targets got a clock-seeded Xorshift.**
//!    `timestamp ^ 0x5DEECE66D` is predictable to anyone who knows
//!    roughly when the process started, and `core.random.secure`
//!    explicitly forbids exactly that substitution.
//!  * **Linux went through libc.** `libc::syscall` violates the
//!    no-libc invariant the interpreter is required to hold
//!    (`docs/architecture/no-libc-architecture.md`); the syscall is
//!    issued directly here.

/// Eight bytes from the platform CSPRNG, or abort.
#[inline]
pub fn secure_random_u64() -> u64 {
    let mut buf = [0u8; 8];
    fill_secure(&mut buf);
    u64::from_ne_bytes(buf)
}

/// Fill `buf` from the platform CSPRNG, or abort.
///
/// Blocks if the kernel pool is not yet initialised — which is
/// correct, and the reason `GRND_NONBLOCK` is not passed: early-boot
/// code that would rather have weak randomness than wait does not
/// exist in this runtime.
pub fn fill_secure(buf: &mut [u8]) {
    if let Err(code) = try_fill_secure(buf) {
        entropy_unavailable(code);
    }
}

/// Fill `buf` from the platform CSPRNG, reporting failure.
///
/// `Err` carries the platform's own code — a negative errno on Linux,
/// the `errno` value on Darwin, an NTSTATUS on Windows — so a caller
/// that surfaces it to Verum can build a faithful `OSError`.
pub fn try_fill_secure(buf: &mut [u8]) -> Result<(), i64> {
    if buf.is_empty() {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        let mut filled = 0usize;
        while filled < buf.len() {
            let n = unsafe { getrandom_raw(buf[filled..].as_mut_ptr(), buf.len() - filled) };
            match n {
                // EINTR: interrupted before any bytes were written.
                -4 => continue,
                n if n > 0 => filled += n as usize,
                other => return Err(other),
            }
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        // `getentropy` is capped at 256 bytes per call by POSIX.
        for chunk in buf.chunks_mut(256) {
            // SAFETY: `chunk` is a live, writable slice of `chunk.len()`
            // bytes and the length is <= 256, the documented maximum.
            // libSystem is the entropy boundary the architecture doc
            // permits on Darwin.
            if unsafe { getentropy(chunk.as_mut_ptr(), chunk.len()) } != 0 {
                return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(-1) as i64);
            }
        }
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        // SAFETY: `buf` is a live, writable slice of `buf.len()` bytes.
        // A null algorithm handle is what BCRYPT_USE_SYSTEM_PREFERRED_RNG
        // requires.
        let status = unsafe {
            windows_sys::Win32::Security::Cryptography::BCryptGenRandom(
                std::ptr::null_mut(),
                buf.as_mut_ptr(),
                buf.len() as u32,
                0x0000_0002u32, // BCRYPT_USE_SYSTEM_PREFERRED_RNG
            )
        };
        if status == 0 { Ok(()) } else { Err(status as i64) }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = buf;
        // No entropy source is claimed for this target. Inventing one
        // is the defect this module exists to remove. -ENOSYS.
        Err(-38)
    }
}

/// No usable entropy: report precisely and stop.
///
/// Returning would mean handing back the zeroed buffer, which is the
/// failure this module exists to prevent, so this never returns.
#[cold]
#[inline(never)]
fn entropy_unavailable(code: i64) -> ! {
    eprintln!(
        "verum: the platform CSPRNG is unavailable (code {code}). \
         Refusing to continue: returning predictable bytes from a \
         secure-randomness request would compromise every key, nonce \
         and token derived from it."
    );
    std::process::abort();
}

/// `getrandom(2)` issued directly, without libc.
///
/// Returns the raw kernel result: the byte count on success, or a
/// negative errno.
///
/// # Safety
/// `dst` must be valid for writes of `len` bytes.
#[cfg(target_os = "linux")]
#[inline]
unsafe fn getrandom_raw(dst: *mut u8, len: usize) -> i64 {
    let ret: i64;
    #[cfg(target_arch = "x86_64")]
    // SAFETY: the syscall writes at most `len` bytes to `dst`, which
    // the caller guarantees is valid for that many writes. `~{memory}`
    // (implied by the default `options`) keeps the compiler from
    // caching those bytes across the call.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") verum_common::linux_syscalls::x86_64::SYS_GETRANDOM as i64 => ret,
            in("rdi") dst,
            in("rsi") len,
            in("rdx") 0,           // flags: blocking draw
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }
    #[cfg(target_arch = "aarch64")]
    // SAFETY: as above.
    unsafe {
        core::arch::asm!(
            "svc #0",
            in("x8") verum_common::linux_syscalls::aarch64::SYS_GETRANDOM as i64,
            inlateout("x0") dst => ret,
            in("x1") len,
            in("x2") 0,
            options(nostack)
        );
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        let _ = (dst, len);
        // No direct-syscall sequence is written for this architecture,
        // so no entropy is claimed for it. -ENOSYS.
        ret = -38;
    }
    ret
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    /// libSystem; POSIX 2024. Returns 0, or -1 with `errno` set.
    fn getentropy(buf: *mut u8, len: usize) -> i32;
}
