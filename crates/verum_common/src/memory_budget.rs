//! A memory ceiling the process enforces on ITSELF.
//!
//! # Why this exists
//!
//! On macOS `ulimit -v` does not work. It reports
//! `setrlimit failed: invalid argument`, leaves the limit `unlimited`,
//! and a process then allocates freely past the number that was asked
//! for — verified: 700 MB allocated under a "512 MB" limit. So the
//! usual outer guard is not available on the platform this compiler is
//! developed on, and a runaway allocation takes the whole machine with
//! it: 128 GB of RAM exhausted, swap full, the desktop unusable until
//! something is killed by hand. That has happened repeatedly.
//!
//! A process cannot rely on its environment to stop it, so it stops
//! itself. This allocator counts live bytes and aborts with a legible
//! message when the total crosses a ceiling — a crash with a reason
//! instead of a machine that has to be power-cycled.
//!
//! # What the ceiling is not
//!
//! It is not a fix for whatever allocates too much; it is the seatbelt.
//! The number it prints — the peak at the moment of death — is the
//! first fact any such investigation needs, and it is exactly what an
//! OOM-killed process fails to leave behind.
//!
//! # Use
//!
//! ```ignore
//! use verum_common::memory_budget::BudgetedAllocator;
//!
//! #[global_allocator]
//! static ALLOC: BudgetedAllocator = BudgetedAllocator::new();
//! ```
//!
//! The ceiling comes from `VERUM_MEMORY_CEILING_GB`, defaulting to
//! [`DEFAULT_CEILING_GB`]. `VERUM_MEMORY_CEILING_GB=0` disables the
//! check for a deliberate large run.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Ceiling used when `VERUM_MEMORY_CEILING_GB` is unset.
///
/// Chosen against the measured peak of the heaviest thing this
/// toolchain does — the stdlib bake, which peaks around 8 GB — with
/// room for growth, and far below the point where a 128 GB machine
/// starts swapping to death.
pub const DEFAULT_CEILING_GB: usize = 24;

pub const BYTES_PER_GB: usize = 1024 * 1024 * 1024;
const BYTES_PER_MB: usize = 1024 * 1024;

/// Exit status when the ceiling stops the process.
///
/// Distinct from any status the compiler itself returns, so a wrapper
/// script can tell "ran out of room" from "rejected the program".
pub const EXIT_CODE_MEMORY_CEILING: i32 = 101;

static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);
static PEAK_BYTES: AtomicUsize = AtomicUsize::new(0);
static ABORTING: AtomicBool = AtomicBool::new(false);

/// Live bytes currently allocated through the budgeted allocator.
pub fn live_bytes() -> usize {
    LIVE_BYTES.load(Ordering::Relaxed)
}

/// The high-water mark since process start.
pub fn peak_bytes() -> usize {
    PEAK_BYTES.load(Ordering::Relaxed)
}

/// Sentinel meaning "not read yet" — a real ceiling is never `usize::MAX`.
const CEILING_UNSET: usize = usize::MAX;
static CEILING: AtomicUsize = AtomicUsize::new(CEILING_UNSET);

/// The ceiling in bytes, read from the environment WITHOUT allocating.
///
/// This is called from inside `alloc`, and the obvious spelling
/// deadlocks: `std::env::var` returns a `String`, so it allocates, so
/// it re-enters the allocator, which calls this again, which blocks on
/// the `OnceLock` the outer call is still initialising. Measured — the
/// probe hung before printing its first line, three separate fixes to
/// unrelated suspicions later.
///
/// `getenv` hands back a borrowed C string and allocates nothing, and a
/// plain `AtomicUsize` cannot deadlock. A benign race where two threads
/// both parse and store the same value is fine.
fn ceiling_bytes() -> usize {
    let cached = CEILING.load(Ordering::Relaxed);
    if cached != CEILING_UNSET {
        return cached;
    }
    // Megabytes win when both are set: the finer unit is the more
    // specific request. It exists so the ceiling can be TESTED — the
    // smallest expressible GB ceiling is 1 GB, and a test that must
    // allocate a gigabyte to watch the seatbelt fasten is a test that
    // hangs on a loaded machine instead of failing.
    let value = match read_env_usize(c"VERUM_MEMORY_CEILING_MB") {
        Some(mb) => mb.saturating_mul(BYTES_PER_MB),
        None => read_env_usize(c"VERUM_MEMORY_CEILING_GB")
            .unwrap_or(DEFAULT_CEILING_GB)
            .saturating_mul(BYTES_PER_GB),
    };
    CEILING.store(value, Ordering::Relaxed);
    value
}

/// Reads a non-negative decimal environment variable without allocating.
///
/// Returns `None` when unset, empty, or not entirely digits — a
/// malformed ceiling should fall back to the default rather than
/// silently become zero, which would disable the guard.
fn read_env_usize(name: &std::ffi::CStr) -> Option<usize> {
    // SAFETY: `getenv` returns a pointer into the process environment,
    // valid until the environment is modified; it is read immediately
    // and never retained. Nothing here allocates. The value is walked
    // as bytes, so the canonical `*mut c_char` is viewed as `*const u8`.
    let raw = unsafe { libc_getenv(name.as_ptr()) }.cast_const().cast::<u8>();
    if raw.is_null() {
        return None;
    }
    let mut value: usize = 0;
    let mut seen_digit = false;
    let mut at = 0isize;
    loop {
        // SAFETY: walking a NUL-terminated C string returned by getenv.
        let byte = unsafe { *raw.offset(at) };
        if byte == 0 {
            break;
        }
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value
            .checked_mul(10)?
            .checked_add((byte - b'0') as usize)?;
        seen_digit = true;
        at += 1;
    }
    seen_digit.then_some(value)
}

/// Writes the ceiling message and ends the process WITHOUT allocating.
///
/// This runs from inside `alloc`, and everything convenient is
/// forbidden there:
///
/// * `eprintln!` allocates (formatting machinery, the stderr lock's
///   line buffer) — so calling it re-enters the allocator that is
///   already mid-call. Measured: the test for this guard hung past 60
///   seconds, twice, until the formatting came out.
/// * `std::process::exit` runs `atexit` handlers, which flush buffers
///   and allocate, with the same result.
/// * `std::process::abort` on macOS hands the corpse to ReportCrash,
///   which writes a crash report under the memory pressure that just
///   triggered this — and leaves `.ips` files behind.
///
/// So: format into a stack buffer with integer arithmetic, write it
/// with one `write(2)`, and leave through `_exit(2)`, which skips
/// handlers entirely.
fn report_and_exit(live: usize, ceiling: usize) -> ! {
    // Megabytes, in integers — no float formatting machinery, no
    // allocation. MB rather than fractional GB because the first
    // version printed a 64 MB ceiling as "0.6 GB": two decimals of a
    // gigabyte cannot say small numbers, and the number this prints is
    // the one somebody will act on.
    let mb = |b: usize| b / BYTES_PER_MB;

    let mut buf = [0u8; 512];
    let mut at = 0usize;
    let put = |bytes: &[u8], buf: &mut [u8; 512], at: &mut usize| {
        let room = buf.len().saturating_sub(*at);
        let n = bytes.len().min(room);
        buf[*at..*at + n].copy_from_slice(&bytes[..n]);
        *at += n;
    };
    let put_num = |mut v: usize, buf: &mut [u8; 512], at: &mut usize| {
        let mut digits = [0u8; 20];
        let mut n = 0;
        loop {
            digits[n] = b'0' + (v % 10) as u8;
            v /= 10;
            n += 1;
            if v == 0 {
                break;
            }
        }
        while n > 0 {
            n -= 1;
            if *at < buf.len() {
                buf[*at] = digits[n];
                *at += 1;
            }
        }
    };

    put(b"\nverum: MEMORY CEILING REACHED - stopping instead of exhausting the machine.\n  live    ", &mut buf, &mut at);
    put_num(mb(live), &mut buf, &mut at);
    put(b" MB\n  ceiling ", &mut buf, &mut at);
    put_num(mb(ceiling), &mut buf, &mut at);
    put(
        b" MB  (VERUM_MEMORY_CEILING_GB, or _MB for a finer one; 0 disables)\n\
          \nThis is a seatbelt, not a diagnosis: something allocated far more than\n\
          this toolchain's heaviest measured step (the stdlib bake, ~8 GB peak).\n",
        &mut buf,
        &mut at,
    );

    // SAFETY: `write` to fd 2 with a pointer/length pair from a live
    // stack buffer; `_exit` never returns. Neither touches the
    // allocator, which is the entire point.
    unsafe {
        let mut written = 0usize;
        while written < at {
            let n = libc_write(2, buf.as_ptr().add(written).cast(), at - written);
            if n <= 0 {
                break;
            }
            written += n as usize;
        }
        libc_exit(EXIT_CODE_MEMORY_CEILING)
    }
}

// The two syscalls this needs, declared directly rather than pulling in
// a dependency for them. Both are in libSystem/libc on every platform
// this compiler is hosted on.
// Signatures match libc's canonical prototypes exactly — rustc's
// `suspicious_runtime_symbol_definitions` lint checks declarations of
// runtime symbols the standard library itself uses (`write`) against
// those prototypes, and a `*const u8` where libc says `*const c_void`
// is a hard error under `-D warnings`.
unsafe extern "C" {
    #[link_name = "write"]
    fn libc_write(fd: i32, buf: *const std::ffi::c_void, count: usize) -> isize;
    #[link_name = "_exit"]
    fn libc_exit(code: i32) -> !;
    #[link_name = "getenv"]
    fn libc_getenv(name: *const std::ffi::c_char) -> *mut std::ffi::c_char;
}

/// A `System` allocator that counts, and refuses to help the process
/// exhaust the machine.
pub struct BudgetedAllocator;

impl Default for BudgetedAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl BudgetedAllocator {
    pub const fn new() -> Self {
        BudgetedAllocator
    }

    #[inline]
    fn account(&self, added: usize) {
        let live = LIVE_BYTES.fetch_add(added, Ordering::Relaxed) + added;
        // `fetch_max` keeps the peak truthful under threads without a
        // lock; a slightly stale peak is acceptable, a torn one is not.
        PEAK_BYTES.fetch_max(live, Ordering::Relaxed);

        let ceiling = ceiling_bytes();
        if ceiling == 0 || live <= ceiling {
            return;
        }
        // One thread reports; the rest would only interleave their
        // messages into the same stderr while the machine is already in
        // trouble.
        if ABORTING.swap(true, Ordering::SeqCst) {
            return;
        }
        report_and_exit(live, ceiling);
    }
}

// SAFETY: every method forwards to `System`, which satisfies the
// `GlobalAlloc` contract; the added work is accounting on atomics and
// never touches the returned pointers.
unsafe impl GlobalAlloc for BudgetedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            self.account(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            self.account(layout.size());
        }
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            if new_size >= layout.size() {
                self.account(new_size - layout.size());
            } else {
                LIVE_BYTES.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new_ptr
    }
}
