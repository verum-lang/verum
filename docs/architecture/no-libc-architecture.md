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
| `runtime.rs::get_or_declare_open`             | `open`                | Linux `SYS_open` (256 / 56) ; libSystem on macOS. **Open.** |
| ✅ `runtime.rs::get_or_declare_close`         | `close`               | Linux `SYS_close` (3) direct syscall ; libSystem on macOS. **Closed (commit pending).** |
| ✅ `runtime.rs::get_or_declare_read`          | `read`                | Linux `SYS_read` (0) direct syscall ; libSystem on macOS. **Closed.** |
| ✅ `runtime.rs::get_or_declare_write`         | `write`               | Linux `SYS_write` (1) direct syscall ; libSystem on macOS. **Closed.** |
| ✅ `runtime.rs::get_or_declare_strlen`        | `strlen`              | Open-coded null-byte scan loop emitted in IR (no symbol). **Closed.** |
| ✅ `runtime.rs::get_or_declare_memcpy`        | `memcpy`              | Internal-linkage wrapper over `llvm.memcpy.p0.p0.i64`. **Closed.** |
| ✅ `runtime.rs::get_or_declare_memset`        | `memset`              | Internal-linkage wrapper over `llvm.memset.p0.i64`. **Closed.** |
| `runtime.rs::get_or_declare_malloc`           | `malloc`              | `core/mem/allocator.vr`'s mmap-backed arena. **Open.** |
| ✅ `runtime.rs::get_or_declare_unlink`        | `unlink`              | Linux x86_64 `SYS_unlink` (87) / aarch64 `SYS_unlinkat` (35) ; libSystem on macOS. **Closed.** |
| ✅ `runtime.rs::get_or_declare_lseek`         | `lseek`               | Linux `SYS_lseek` (8) direct syscall ; libSystem on macOS. **Closed.** |
| ✅ `runtime.rs::get_or_declare_access`        | `access`              | Linux x86_64 `SYS_access` (21) / aarch64 `SYS_faccessat` (48) ; libSystem on macOS. **Closed.** |
| `runtime.rs::get_or_declare_clock_gettime`    | `clock_gettime`       | Already replaced for Linux / macOS via direct syscall + libSystem; the helper itself remains for the macOS + other-Unix fallback paths.  Audit each remaining call. |
| `runtime.rs::get_or_declare_nanosleep`        | `nanosleep`           | Same as clock_gettime.                       |
| `runtime.rs::get_or_declare_freeaddrinfo`     | `freeaddrinfo`        | Audit usage; the resolver path likely shouldn't require this in a no-libc world. |
| `platform_ir.rs::emit_socket_*` (~10 helpers) | `setsockopt` etc.     | Direct syscall / WinSock — partial in progress. |
| `verum_vbc::ffi::*` (libffi paths)            | All libc syscalls     | Replace with `__sys_*_raw` intrinsics that bypass libffi. |

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
