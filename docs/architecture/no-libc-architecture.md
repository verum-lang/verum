# No-libc Architecture Rule

**Status: load-bearing architectural invariant**.  Per user directive
2026-05-01: «лучше вообще удалить весь код имеющий отношение к libc —
чтобы не было путаницы и чётко прописать что interpreter+aot у нас не
должен использовать libc».

## The rule

Verum's two execution paths — the **VBC interpreter** (Tier 0) and
the **AOT-compiled binary** (Tier 1) — MUST NOT call into libc.
This applies to every artefact the user runs — `verum run` (Tier 0
interpreter), `verum build` + execution of the produced `.exe`
(Tier 1 AOT), and any shared library / object file emitted by the
codegen pipeline.

## Per-platform replacement strategy

| Target  | Replacement for libc                                |
|---------|-----------------------------------------------------|
| Linux   | Direct syscalls via `syscall` / `svc #0` instruction. |
| macOS   | libSystem.dylib (Apple's required system interface; not "libc" in glibc/musl sense — Apple prohibits direct syscalls and `libSystem` is the minimum boundary). |
| Windows | `kernel32.dll` + `ntdll.dll` (no MSVC CRT, no UCRT). |
| FreeBSD | Direct syscalls via `int 0x80` / `syscall`.         |
| Embedded| Bare-metal — no OS dependencies at all.             |

The macOS and embedded paths are *exceptions by necessity* — Apple's
ABI requires libSystem, and embedded has no OS to ask.  Every other
path must hit kernel facilities directly.

## What this rules out

* No `extern "C" fn open`, `read`, `write`, `close` declarations in
  emitted LLVM IR (or in any `core/sys/<plat>/*.vr` for the Tier 0
  interpreter side).  Use the platform-specific direct-syscall
  intrinsic instead.
* No `extern "C" fn malloc`, `free`, `calloc`, `realloc`.  The
  allocator lives in `core/mem/allocator.vr` and uses `mmap` /
  `VirtualAlloc` directly.
* No `__error` / `__errno_location` indirection.  Errno is a
  per-platform syscall convention — Linux returns `-errno` from the
  syscall instruction, Windows uses `GetLastError`, macOS uses
  `__error`.  Codegen emits the appropriate primitive based on the
  *target* triple (NOT host `#[cfg]`).
* No `nanosleep`, `clock_gettime`, `getpid`, `getrandom`,
  `getentropy`, `gettid` libc wrappers.  Direct syscalls.
* No libc string/byte intrinsics — `memcpy`, `memset`, `memcmp`,
  `strlen`, `strcmp`.  These ALL have LLVM intrinsic forms
  (`llvm.memcpy.p0.i64`, `llvm.memset.p0.i64`, etc.) that are
  *NOT* libc — LLVM's `MemCpyOptPass` / backend lowers them to
  inline asm or an equivalent native code sequence.  Use
  `verum_codegen::llvm::ffi::FfiLowering::lower_memset` (and
  friends) which emit the intrinsic, never the libc symbol.
* No `socket` / `bind` / `listen` / `accept` / `connect` libc
  declarations — TCP/UDP go through the v2 intrinsic family
  (`__tcp_listen_v2_raw`, `__tcp_accept_raw`, etc.) which
  themselves dispatch to direct syscalls (Linux) or libSystem
  (macOS).
* No `pthread_*` declarations (other than macOS's
  `pthread_threadid_np` which IS in libSystem and acceptable).
  Threading goes through Linux `clone3` / Windows `CreateThread` /
  macOS pthread (libSystem path).

## Verification

CI must confirm no libc:

```bash
# Linux: no libc.so.6 / libgcc / libpthread references.
ldd target/release/<bin>
# Expected output (Linux):
#   linux-vdso.so.1 (...)
#   /lib64/ld-linux-x86-64.so.2 (...)
# That's it — only the dynamic linker.

# macOS: only libSystem (acceptable).
otool -L target/release/<bin>
# Expected output:
#   /usr/lib/libSystem.B.dylib

# Windows: only kernel32 + ntdll.
dumpbin /imports <bin>.exe
# Expected: kernel32.dll, ntdll.dll only.
```

A CI gate that runs this check on every release artifact closes the
loop.

## Current state (2026-05-01)

Migration is **in progress**.  Already libc-free:

* Linux direct syscalls for `clock_gettime` (monotonic + realtime),
  `nanosleep`, `getpid`, `gettid` — `runtime.rs::emit_verum_time_*`
  + `emit_verum_sys_*`.
* TCP listener / accept / send / recv / close — `__tcp_listen_v2_raw`
  family in `verum_vbc::intrinsics`.
* Cryptographic zeroise — `lower_secure_zero` emits volatile
  `llvm.memset` intrinsic, not libc memset.  Audit at
  `tls-quic-security-audit.md` §2 Action #2 closed.
* MakeVariantTyped placeholder safety gate — variant codegen no
  longer emits invalid `MakeVariantTyped` against placeholder
  type descriptors (commit 064ea429).
* Cross-compilation-correct codegen: every per-platform decision
  in `runtime.rs` / `platform_ir.rs` reads `module.get_triple()`,
  not host `#[cfg(target_os = "...")]`.

Remaining libc surface (load-bearing punch list — close before
shipping):

| File                                          | Symbol(s)             | Replacement                                  |
|-----------------------------------------------|-----------------------|----------------------------------------------|
| ✅ `runtime.rs::get_or_declare_open`          | `open`                | Linux x86_64 `SYS_open` (2) / aarch64 `SYS_openat` (56) ; libSystem on macOS.  Variadic ABI bug closed by construction (fixed 3-arg wrapper). **Closed.** |
| ✅ `runtime.rs::get_or_declare_close`         | `close`               | Linux `SYS_close` (3) direct syscall ; libSystem on macOS. **Closed (commit pending).** |
| ✅ `runtime.rs::get_or_declare_read`          | `read`                | Linux `SYS_read` (0) direct syscall ; libSystem on macOS. **Closed.** |
| ✅ `runtime.rs::get_or_declare_write`         | `write`               | Linux `SYS_write` (1) direct syscall ; libSystem on macOS. **Closed.** |
| ✅ `runtime.rs::get_or_declare_strlen`        | `strlen`              | Open-coded null-byte scan loop emitted in IR (no symbol). **Closed.** |
| ✅ `runtime.rs::get_or_declare_memcpy`        | `memcpy`              | Internal-linkage wrapper over `llvm.memcpy.p0.p0.i64`. **Closed.** |
| ✅ `runtime.rs::get_or_declare_memset`        | `memset`              | Internal-linkage wrapper over `llvm.memset.p0.i64`. **Closed.** |
| ✅ `runtime.rs::get_or_declare_malloc`        | `malloc`              | Wrapper `verum_checked_malloc` routes through `verum_os_alloc` (mmap on Linux/macOS, VirtualAlloc on Windows) + `verum_os_exit` for OOM abort. **Closed.** |
| ✅ `ffi.rs::get_or_declare_malloc/free/realloc` | `malloc`/`free`/`realloc` | malloc → `verum_os_alloc`; free → `verum_internal_free` (no-op stub; CBGR epoch model handles bulk invalidation); realloc → `verum_internal_realloc` (allocate-new wrapper). **Closed.** |
| ✅ `instruction.rs::checked_malloc_instr`     | `malloc` + `_exit`    | Both routed through `verum_os_alloc` and `verum_os_exit`. **Closed.** |
| ✅ `instruction.rs` strcmp call sites (×3)    | `strcmp`              | Single shared `verum_internal_strcmp` helper — inline byte-by-byte comparison loop with null-termination check.  Internal-linkage so the symbol doesn't escape. **Closed.** |
| ✅ `instruction.rs` puts call sites (×9)      | `puts`                | Single shared `verum_internal_puts` helper that calls `verum_internal_strlen` + `verum_internal_write` (both libc-free) plus a trailing newline.  Internal-linkage. **Closed.** |
| ✅ All `_exit` call sites (×13+ across instruction.rs / ffi.rs / platform_ir.rs / runtime.rs) | `_exit` | Bulk-renamed to `verum_internal_exit_i64` — internal-linkage wrapper that truncates i64→i32 and calls `verum_os_exit` (which itself uses ExitProcess on Windows, `_exit` syscall on Linux, libSystem `_exit` on macOS). **Closed.** |
| ✅ `runtime.rs` `free` call sites (×13 declarations + lookups) | `free` | Bulk-renamed to `verum_internal_free` — wrapper defined in `ffi.rs::get_or_declare_free` (no-op stub since CBGR's epoch model handles bulk invalidation; explicit per-pointer free is rarely on the hot path). **Closed.** |
| ✅ `runtime.rs` `calloc` call sites (×4) | `calloc` | New `define_internal_calloc` helper computes `n*size` then routes through `verum_os_alloc` (mmap-based — pages are MAP_ANONYMOUS-zeroed, so calloc's zero-init contract is satisfied without an explicit memset).  All 4 call sites + 2 declaration sites updated. **Closed.** |
| ✅ `runtime.rs` orphan `malloc` + inline libc `strlen` / `memcpy` declarations (×3) | `malloc`/`strlen`/`memcpy` | Re-pointed to `verum_os_alloc` / `verum_internal_strlen` / `verum_internal_memcpy` (all libc-free wrappers). **Closed.** |
| ✅ `runtime.rs::get_or_declare_unlink`        | `unlink`              | Linux x86_64 `SYS_unlink` (87) / aarch64 `SYS_unlinkat` (35) ; libSystem on macOS. **Closed.** |
| ✅ `runtime.rs::get_or_declare_lseek`         | `lseek`               | Linux `SYS_lseek` (8) direct syscall ; libSystem on macOS. **Closed.** |
| ✅ `runtime.rs::get_or_declare_access`        | `access`              | Linux x86_64 `SYS_access` (21) / aarch64 `SYS_faccessat` (48) ; libSystem on macOS. **Closed.** |
| `runtime.rs::get_or_declare_clock_gettime`    | `clock_gettime`       | Already replaced for Linux / macOS via direct syscall + libSystem; the helper itself remains for the macOS + other-Unix fallback paths.  Audit each remaining call. |
| `runtime.rs::get_or_declare_nanosleep`        | `nanosleep`           | Same as clock_gettime.                       |
| `runtime.rs::get_or_declare_freeaddrinfo`     | `freeaddrinfo`        | DNS resolver path.  Strategy: replace `getaddrinfo`/`freeaddrinfo` pair with a Verum-native DNS resolver that talks UDP to nameservers from `/etc/resolv.conf` directly.  Substantial work — ~500 LOC.  **Deferred — large standalone task.** |
| `runtime.rs::get_or_declare_inet_pton`        | `inet_pton`           | IP-string parsing.  Strategy: emit inline parser for AF_INET (4-byte) and AF_INET6 (16-byte) cases.  ~80 LOC of LLVM IR.  **Open — straightforward but pending.** |
| ✅ `platform_ir.rs::ensure_networking_syscalls` (×11 socket family) | `socket`/`connect`/`bind`/`listen`/`accept`/`send`/`recv`/`sendto`/`recvfrom`/`setsockopt`/`waitpid` | Refactored from `extern "C"` declarations to internal-linkage wrappers via new `emit_libc_free_socket_wrapper` helper.  Linux dispatch via direct syscalls with arch-correct numbers (x86_64: socket=41, connect=42, accept=43, sendto=44, recvfrom=45, bind=49, listen=50, setsockopt=54, waitpid=61; aarch64: socket=198, connect=203, accept=202, sendto=206, recvfrom=207, bind=200, listen=201, setsockopt=208, waitpid=260).  macOS routes through `__verum_libsys_*` indirection to libSystem.  **Closed.** |
| `verum_vbc::ffi::*` (libffi paths)            | All libc syscalls     | Replace with `__sys_*_raw` intrinsics that bypass libffi.  Substantial work — ~1000 LOC across `verum_vbc/src/ffi/platform/{linux,darwin}.rs`.  **Deferred — large standalone task.** |
| **Internal-only paths (debug/test, not user-facing):** | | |
| `instruction.rs` printf (×3 in debug helper)  | `printf`              | Debug helper for `Debug` opcode.  Strategy: route through `verum_internal_puts` after formatting i64/f64 → text via Verum's own number-to-text helpers.  Lower priority since debug-mode-only. **Open — internal only.** |
| `instruction.rs` strtol (×2)                  | `strtol`              | Verum text → int parser.  ~30 LOC of LLVM IR (skip whitespace, optional sign, accumulate digits).  **Open — straightforward.** |
| `instruction.rs` strtod (×2)                  | `strtod`              | Float parsing — genuinely complex (Ryu, exponents, NaN/Inf).  Could route through libSystem on macOS (acceptable) and require migration on Linux only.  **Open — large effort for Linux path.** |
| `platform_ir.rs::emit_exception_handling`     | `setjmp`/`longjmp`    | Exception unwinding primitive.  No direct syscall — purely CPU register save/restore.  Strategy: emit `llvm.eh.sjlj.setjmp` LLVM intrinsic (lowers to inline asm, not libc).  **Open — special-case.** |
| `verum_kernel` / Rust internals               | Rust stdlib's libc usage | Not in scope — Verum compiler/host concerns; the *produced binary* is the audit target. |

## Why this matters

* **Reproducibility**: A binary that links libc inherits libc's
  versioning (glibc 2.31 vs 2.35), security posture, and ABI churn.
  Eliminating libc means a Verum binary built today runs on any
  kernel from the lifetime of the Verum-supported syscall set —
  forever, no `GLIBC_2.34: not found` errors.
* **Security**: libc is a large attack surface.  Verum's own runtime
  is auditable in isolation; libc is not.  Removing it reduces the
  attack surface to just the kernel ABI.
* **Performance**: Direct syscalls skip the libc-side wrapper
  (typically 5-15 ns per call), errno-thread-local indirection,
  and call-site glue.
* **Scientific honesty**: Verum claims to be a from-first-principles
  language.  A binary that depends on libc *isn't* — every libc
  call drags in C's invariants and bug catalogue.  The "no libc"
  rule keeps the claim load-bearing.

## Owner / mechanism

* Owner: codegen (verum_codegen) + runtime (verum_vbc) maintainers.
* Mechanism: every PR that adds an `extern "C"` declaration in IR
  emission code or a `@intrinsic` dispatch path must justify the
  symbol against this document.  Reviewers reject unless the symbol
  is in the macOS-libSystem allow-list (acceptable per Apple ABI)
  or the embedded-bare-metal allow-list.
