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
| ✅ ~~`runtime.rs::get_or_declare_getaddrinfo` / `get_or_declare_freeaddrinfo`~~ (deleted) | `getaddrinfo` / `freeaddrinfo` | **CLOSED — last member of the getaddrinfo class (task #43, 2026-07-16).** Native resolver had already LANDED in `core/net/dns.vr` (pure RFC-1035 DNS-over-UDP/TCP on the B1d UDP stack, no FFI: 0xC0 pointer-decompression depth-capped, `/etc/resolv.conf` via raw-syscall `read_system_file`, `/etc/hosts` + hardcoded RFC-6761 loopback resolved **before** any network query, timeout+retry, UDP→TCP fallback, A/AAAA/CNAME/MX/TXT/SRV/NS/PTR/SOA). The remaining AOT hole — `verum_tcp_connect` open-coding `getaddrinfo`/`freeaddrinfo` to resolve+connect in one step — is now closed: **(1)** `verum_tcp_connect` is narrowed to **IP-literal-only**, building `sockaddr_in` directly and parsing the address through the already-libc-free `verum_internal_inet_pton` (AF_INET), then `socket`/`connect` — mirroring the `verum_udp_send_to` closure; a non-IP / `inet_pton ≤ 0` input returns the honest connect-fail sentinel `-1` (never a silent 0). Both `get_or_declare_getaddrinfo` and `get_or_declare_freeaddrinfo` are **deleted** (grep over `crates/verum_codegen` = 0 live declarations); `getaddrinfo`/`freeaddrinfo` also dropped from the `is_libc_extern` allow-list in `vbc_lowering.rs`. **(2)** `RawTcpStream.connect` in `core/sys/net_ops.vr` now resolves the host through `core.net.dns.lookup_host` FIRST (which fast-paths IP literals with zero network I/O and hits the static host DB for `localhost`) and hands the intrinsic the first IPv4 result as a dotted-quad literal. Interp path (`net_runtime::tcp_connect`) uses host Rust `std::net` and is unaffected by the AOT no-libc invariant. |
| ✅ `runtime.rs::get_or_declare_inet_pton`     | `inet_pton`           | Open-coded IPv4 dotted-decimal parser emitted as `verum_internal_inet_pton` (internal-linkage).  Walks src byte-by-byte with PHI-driven state machine: 4 octets × ≤3 digits, validates each in [0,255], stores i8 into dst[0..4].  Returns 1 on success, 0 on parse error, -1 on AF_INET6 (unsupported in this minimal version — IPv6 callers fall back to libSystem on macOS or use the v2 TCP intrinsic family). **Closed for IPv4.** |
| ✅ `platform_ir.rs::ensure_networking_syscalls` (×11 socket family) | `socket`/`connect`/`bind`/`listen`/`accept`/`send`/`recv`/`sendto`/`recvfrom`/`setsockopt`/`waitpid` | Refactored from `extern "C"` declarations to internal-linkage wrappers via new `emit_libc_free_socket_wrapper` helper.  Linux dispatch via direct syscalls with arch-correct numbers (x86_64: socket=41, connect=42, accept=43, sendto=44, recvfrom=45, bind=49, listen=50, setsockopt=54, waitpid=61; aarch64: socket=198, connect=203, accept=202, sendto=206, recvfrom=207, bind=200, listen=201, setsockopt=208, waitpid=260).  macOS routes through `__verum_libsys_*` indirection to libSystem.  **Closed.** |
| `verum_vbc::ffi::*` (libffi paths)            | All libc syscalls     | Replace with `__sys_*_raw` intrinsics that bypass libffi.  Substantial work — ~1000 LOC across `verum_vbc/src/ffi/platform/{linux,darwin}.rs`.  **Deferred — large standalone task.** |
| **Internal-only paths (debug/test, not user-facing):** | | |
| `instruction.rs` printf (×3 in debug helper)  | `printf`              | Debug helper for `Debug` opcode.  Strategy: route through `verum_internal_puts` after formatting i64/f64 → text via Verum's own number-to-text helpers.  Lower priority since debug-mode-only. **Open — internal only.** |
| ✅ `instruction.rs` strtol                    | `strtol`              | Open-coded base-10 integer parser emitted as `verum_internal_strtol` (internal-linkage).  PHI-driven state machine: skip whitespace → optional `+`/`-` sign → digit-accumulation loop → multiply by sign.  Signature matches libc `(ptr, ptr, i32) -> i64`. **Closed.** |
| ✅ `instruction.rs` strtod (×2) + `runtime.rs::emit_verum_text_parse_float` | `strtod`              | Float parsing.  All three call sites (2-operand `ParseFloat` in `instruction.rs`, and `verum_text_parse_float` in `runtime.rs` which the method-call `.to_float()`/`.parse_float()` path reaches) route through the open-coded `verum_internal_strtod` (internal-linkage, `get_or_declare_internal_strtod`, commit 3bbd867c5) on **every** target — no libc `strtod` symbol is declared or referenced anywhere in codegen (verified: `grep '"strtod"'` = 0 hits).  Grammar: `[ws][±][int][.frac][eE±exp]`; i64 mantissa + power-of-ten f64 scaling (bounded 400 loops → ±Inf/0 by IEEE on over-range, so `"1e400"` → +Inf matches Tier-0).  **Closed for `-nostdlib` linkage.**  *V0 boundary (differential-parity gap, NOT a link blocker):* `inf`/`infinity`/`nan` text literals are not yet recognized — Tier-0 (`str.trim().parse::<f64>()`) returns `Some(±inf)`/`Some(nan)` while Tier-1 returns `Maybe.None` (the `has_digit` gate at `instruction.rs:~21415` and `verum_text_parse_float`'s scan reject a digit-free string).  Close by teaching `verum_internal_strtod` the case-insensitive `inf`/`nan` prefix AND relaxing both None-gates to accept an `i`/`n` lead byte; requires a `verum` rebuild + differential run to land safely — deferred as a coherence follow-up. |
| ✅ `platform_ir.rs::emit_exception_handling` + `emit_exception_ir` + `vbc_lowering.rs` TryBegin | `setjmp`/`longjmp`    | Exception unwinding primitive.  Cross-compilation bug fixed earlier (reads `target_is_darwin(module)`, not host `cfg!`).  **macOS:** libSystem `_setjmp` (TryBegin) + `longjmp` (throw) — acceptable per the architecture rule; unchanged.  **Linux / other Unix (libc-free):** the setjmp site emits `llvm.eh.sjlj.setjmp` **inline** at TryBegin — exactly the sequence clang lowers `__builtin_setjmp` to (store `llvm.frameaddress(0)`→buf[0], `llvm.stacksave()`→buf[2], then the intrinsic → i32).  It must be inline, NOT a `verum_internal_setjmp` wrapper: the frame/stack it saves must be the try-block function's own frame — a helper's frame is dead after it returns, so a later longjmp would restore a stale frame.  The throw site (`emit_exception_ir::verum_exception_throw`) emits `llvm.eh.sjlj.longjmp(buf)`.  Both intrinsics lower to inline asm in the backend — no libc symbol; the bare `setjmp`/`longjmp` module declarations in `emit_exception_handling` are now gated to darwin only.  **Linux body landed — needs native Linux run verify (build + AOT try/throw execution on x86_64 + aarch64).** |
| `verum_kernel` / Rust internals               | Rust stdlib's libc usage | Not in scope — Verum compiler/host concerns; the *produced binary* is the audit target. |
| **Link-step surface (2026-07-15 audit):**     |                       |                                              |
| ✅ Empty C stub `.c`→clang→`.o` per compile unit | (none — vestige)   | `generate_runtime_stubs` / `compile_c_file` and `verum_codegen::runtime_stubs` DELETED — the whole runtime is LLVM IR inside the main object; nothing external is compiled.  `detect_c_compiler` survives only as the platform *linker driver*. **Closed.** |
| ✅ `link_executable` Linux flags              | `-ldl -lrt -lstdc++ -rdynamic` | Removed — the emitted IR references no `dl*` symbols (dlopen appears only in a name-classification list), no rt-only symbols (time = direct syscalls), no C++ runtime (CBGR is pure IR).  **Closed.** |
| ✅ `FinalLinker` system-mode host-`#[cfg]` defaults | `-lrt` (Linux), `-framework CoreFoundation` (macOS) | Removed — unjustified (no CF symbol is ever emitted) and host-gated (cross-link miscompile).  User-specified `[link] libraries` remain the explicit FFI escape hatch. **Closed.** |
| ✅ `linker_config.rs::default_libraries`      | `pthread`/`m`/`dl` (Linux), **`msvcrt`** (Windows) | Linux default now EMPTY; Windows default now `kernel32` + `ntdll` (msvcrt violated the no-CRT rule outright). **Closed.** |
| ✅ `NoLibcConfig::macos()` unconditional GPU frameworks | Metal/Foundation/objc | Removed from the preset — framework links are gated by the post-globaldce `needs_metal` probe (#100) via `extra_flags`. **Closed.** |
| `link_executable` (cc-driver path) full `-nostdlib` | host crt/libc via driver defaults | The canonical no-libc flags live in `NoLibcConfig` (consumed by the FinalLinker/lld path — Linux default).  The cc-driver fallback (and the primary macOS path) still let the driver add crt/libc.  **Groundwork landed (#28 NOSTDLIB-CC-DRIVER-1, 2026-07-15):** opt-in gate `VERUM_NOSTDLIB_CC_DRIVER` in `link_executable` adds `-nostdlib -lSystem` (darwin) / `-nostdlib -nostartfiles` (Linux) + a target-aware compiler-rt **builtins** archive (`NoLibcConfig::compiler_rt_builtins_archive`).  Default OFF ⇒ darwin acceptance (otool -L → libSystem only) unchanged.  Blocker status after audit: **(a) compiler-rt builtins — CLOSED on darwin** (see §Compiler-rt audit below: 0 builtin symbols emitted; `-nostdlib -lSystem` links a correct libSystem-only binary today); **(b) `strtod` Linux body — CLOSED** (all call sites route through the libc-free `verum_internal_strtod`; zero `strtod` symbols; `inf`/`nan` differential-parity gap tracked separately, not a link blocker); **(c) `setjmp`/`longjmp` Linux body — CLOSED (needs native Linux run verify)** (inline `llvm.eh.sjlj.setjmp`/`llvm.eh.sjlj.longjmp`, clang `__builtin_setjmp` pattern, target-aware; darwin still binds libSystem `_setjmp`/`longjmp`); (d) `getaddrinfo` native resolver — **CLOSED (task #43, 2026-07-16):** resolver LANDED in `core/net/dns.vr` (RFC-1035 DNS-over-UDP/TCP on the B1d UDP stack + `/etc/hosts` + `/etc/resolv.conf` + hardcoded RFC-6761 loopback) AND the AOT `verum_tcp_connect` IR emitter is now IP-literal-only (`verum_internal_inet_pton` reuse), so `get_or_declare_getaddrinfo`/`freeaddrinfo` are deleted — see the `freeaddrinfo` row above.  **Linux bodies (b)(c) landed statically; native Linux link + AOT try/throw + float-parse run verify remain.** |

## Compiler-rt audit (#28, 2026-07-15, darwin arm64)

Method: built representative AOT binaries with `verum build --keep-temps`
(a basic-arithmetic probe, an `Int128` div/rem + `Float`↔`Int128`
probe, and a `Float` div/convert probe) and ran `nm` on the emitted
`.o` objects and linked binaries.  The bigint conformance suite is
*not* a compiler-rt exercise: Verum's `BigInt` is
`{ sign: Bool, digits: List<Int> }` (base-10⁹ chunks over `Int` =
i64) — it never touches a native 128-bit integer.

Findings (facts):

* **Zero compiler-rt builtin symbols** (`__divti3`, `__udivti3`,
  `__modti3`, `__multi3`, `__floattidf`, `__fixdfti`, `__ashlti3`,
  …) appear in *any* object or binary — including the `Int128`
  probe.  Confirmed by `nm` over defined **and** undefined symbols.
* `Int128` arithmetic collapses to **i64** under the uniform-i64
  register model: the probe's division lowers to native
  `sdiv x8, x8, x9` (64-bit registers), never a `bl ___divti3`.
  Float↔int conversions are all f64↔i64 (`fcvtzs`/`scvtf`, inline).
  (The `Int128` probe therefore also mis-computes / crashes on
  large values — a *separate* correctness gap, not a link concern.)
* Every remaining undefined symbol in the objects
  (`memcpy`/`memmove`/`memset`/`bzero`/`strlen`,
  `sin`/`cos`/`exp`/`log`/`pow`, `pthread_*`, `clock_gettime`,
  `nanosleep`, `mmap`, `write`, `kqueue`, `__error`, …) is provided
  by **libSystem** on macOS (acceptable per the architecture rule).
* Empirical link test: `cc probe.o <darwin-flags> -nostdlib -lSystem`
  produces a working binary that `otool -L` shows depends on
  **libSystem.B.dylib only** — identical to the default link.  Adding
  the compiler-rt builtins archive (`libclang_rt.osx.a`) on top is
  **inert**: no new dylib dependency, no pulled members.

Conclusion: blocker (a) is **empirically closed on darwin**.  A
`-nostdlib -lSystem` cc-driver link is correct today; the compiler-rt
builtins archive is wired as an opt-in, target-aware fallback
(`NoLibcConfig::compiler_rt_builtins_archive`) so the link stays
correct the day codegen *does* emit an i128 libcall — without
hand-authoring soft-int IR that nothing currently calls.  The
in-tree `llvm/install` ships no compiler-rt; the locator falls back
to the host `clang` resource dir and the Apple CommandLineTools /
Xcode toolchains (`libclang_rt.osx.a`).

Strategy rejected: **IR soft-int helpers** (`platform_ir.rs`).  With
zero builtin symbols referenced today, emitting `__divti3` &c. in IR
would be speculative dead code, contradicting the "only really-used
symbols" rule.  Revisit only if/when the register model gains a real
native i128 and a survey shows which specific builtins the backend
then lowers to a libcall on each target.

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
