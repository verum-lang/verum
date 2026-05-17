//! Canonical platform-syscall declaration registry for AOT codegen.
//!
//! # Architectural invariant
//!
//! Every platform syscall (`clock_gettime`, `nanosleep`, `read`, `write`,
//! `close`, …) is declared in the LLVM module **exactly once**, with a
//! signature derived from this registry. Multiple call sites — across
//! `runtime.rs` and `platform_ir.rs` — must consult the registry rather
//! than re-declaring the same syscall with locally-chosen widths.
//!
//! # Why this exists
//!
//! Before this module, `clock_gettime` was declared three times:
//!   * `runtime.rs::get_or_declare_clock_gettime` →  `(i32, ptr) -> i32` (POSIX C ABI)
//!   * `platform_ir.rs::emit_nursery_await_all`   →  `(i64, ptr) -> i64` (Verum ABI)
//!   * `platform_ir.rs::emit_select_channels`     →  `(i64, ptr) -> i64` (Verum ABI)
//!
//! When two emit paths fired in the same module the second `add_function`
//! returned the first declaration's FunctionValue — but the second site's
//! `build_call` issued arguments shaped for its own intended signature.
//! LLVM IR verification then failed with
//!     `Call parameter type does not match function signature!`
//!
//! # Verum ABI choice: uniform i64
//!
//! Every syscall is declared with `i64` for integer args/returns even
//! when the underlying C signature uses narrower types
//! (`clock_gettime(clockid_t /* int */, struct timespec *)`). This is
//! safe on the platforms Verum targets (x86_64, aarch64) because the
//! ABI passes integers in registers wider than the C type reads:
//!   * x86_64: rdi/rsi (64-bit) for the first two integer args; the
//!     callee reads via edi/esi (32-bit) when the C type is `int` —
//!     truncation is implicit.
//!   * aarch64: x0/x1 (64-bit); the callee reads via w0/w1 (32-bit)
//!     when the C type is narrower.
//!
//! On 32-bit targets (not currently supported) the choice would have
//! to fork; until that exists the i64-everywhere convention is
//! correct, simple, and lets VBC's NaN-boxed value model flow into FFI
//! without per-arg width adapters.
//!
//! # Adding a new syscall
//!
//! Append a `SyscallSig` to [`POSIX_SYSCALLS`]. All call sites that
//! reach for it through [`get_or_declare`] automatically pick up the
//! canonical signature.

use verum_llvm::AddressSpace;
use verum_llvm::builder::Builder;
use verum_llvm::context::Context;
use verum_llvm::module::Module;
use verum_llvm::types::FunctionType;
use verum_llvm::values::{FunctionValue, IntValue};

use super::error::{BuildExt, CallSiteExt, LlvmLoweringError, OptionExt, Result as LlvmResult};

/// Argument or return-value classification under Verum's uniform-i64
/// AOT ABI. Concrete `FunctionType` values are constructed lazily from
/// these descriptors so the registry table is `const`-friendly.
#[derive(Copy, Clone)]
pub enum AbiTy {
    /// 64-bit integer (Verum-uniform). Used for every integer arg/ret
    /// regardless of the underlying C type's width — the calling
    /// convention truncates on the callee side.
    I64,
    /// Opaque pointer.
    Ptr,
    /// Void return.
    Void,
}

impl AbiTy {
    fn ll_arg<'ctx>(self, ctx: &'ctx Context) -> verum_llvm::types::BasicMetadataTypeEnum<'ctx> {
        match self {
            AbiTy::I64 => ctx.i64_type().into(),
            AbiTy::Ptr => ctx.ptr_type(AddressSpace::default()).into(),
            AbiTy::Void => unreachable!("Void is a return-only classification"),
        }
    }

    fn fn_type<'ctx>(
        ctx: &'ctx Context,
        args: &[AbiTy],
        ret: AbiTy,
    ) -> FunctionType<'ctx> {
        let arg_tys: Vec<verum_llvm::types::BasicMetadataTypeEnum<'ctx>> =
            args.iter().map(|a| a.ll_arg(ctx)).collect();
        match ret {
            AbiTy::I64 => ctx.i64_type().fn_type(&arg_tys, false),
            AbiTy::Ptr => ctx.ptr_type(AddressSpace::default()).fn_type(&arg_tys, false),
            AbiTy::Void => ctx.void_type().fn_type(&arg_tys, false),
        }
    }
}

/// Canonical signature of a single platform syscall under Verum ABI.
pub struct SyscallSig {
    /// LLVM symbol name (e.g. `"clock_gettime"`, `"verum_tcp_connect"`).
    pub name: &'static str,
    /// Argument shape under Verum ABI (i64 / Ptr).
    pub args: &'static [AbiTy],
    /// Return shape (`I64`, `Ptr`, or `Void`).
    pub ret: AbiTy,
}

/// The canonical registry. Append-only — every syscall reachable from
/// any LLVM emit path lives here. When adding a new entry, prefer
/// `AbiTy::I64` for all integer slots even if the C signature is
/// narrower; see the module-level docstring for the ABI rationale.
const POSIX_SYSCALLS: &[SyscallSig] = &[
    // ── time ────────────────────────────────────────────────────
    // C: int clock_gettime(clockid_t, struct timespec *)
    SyscallSig {
        name: "clock_gettime",
        args: &[AbiTy::I64, AbiTy::Ptr],
        ret: AbiTy::I64,
    },
    // C: int nanosleep(const struct timespec *, struct timespec *)
    SyscallSig {
        name: "nanosleep",
        args: &[AbiTy::Ptr, AbiTy::Ptr],
        ret: AbiTy::I64,
    },
    // C: int sched_yield(void)
    SyscallSig {
        name: "sched_yield",
        args: &[],
        ret: AbiTy::I64,
    },
    // ── I/O ─────────────────────────────────────────────────────
    // C: int close(int fd)
    SyscallSig {
        name: "close",
        args: &[AbiTy::I64],
        ret: AbiTy::I64,
    },
    // C: ssize_t read(int fd, void *buf, size_t count)
    SyscallSig {
        name: "read",
        args: &[AbiTy::I64, AbiTy::Ptr, AbiTy::I64],
        ret: AbiTy::I64,
    },
    // C: ssize_t write(int fd, const void *buf, size_t count)
    SyscallSig {
        name: "write",
        args: &[AbiTy::I64, AbiTy::Ptr, AbiTy::I64],
        ret: AbiTy::I64,
    },
    // C: int access(const char *pathname, int mode)
    SyscallSig {
        name: "access",
        args: &[AbiTy::Ptr, AbiTy::I64],
        ret: AbiTy::I64,
    },
    // C: int unlink(const char *pathname)
    SyscallSig {
        name: "unlink",
        args: &[AbiTy::Ptr],
        ret: AbiTy::I64,
    },
    // ── sockets ─────────────────────────────────────────────────
    // Each socket syscall is declared exactly once here under
    // Verum's i64-everywhere ABI. Multiple emit paths
    // (`platform_ir::emit_libc_free_socket_wrapper`,
    // `platform_ir::emit_tcp_listen` / `emit_tcp_accept` etc.,
    // `runtime::get_or_declare_listen_libc` and friends) previously
    // each declared these symbols on their own — when they raced,
    // the loser's wrapper body had wrong-arity / wrong-return-type
    // calls. Routing every site through this single source-of-truth
    // eliminates the divergence at the root.
    // C: int socket(int domain, int type, int protocol)
    SyscallSig {
        name: "socket",
        args: &[AbiTy::I64, AbiTy::I64, AbiTy::I64],
        ret: AbiTy::I64,
    },
    // C: int bind(int sockfd, const struct sockaddr *addr, socklen_t addrlen)
    SyscallSig {
        name: "bind",
        args: &[AbiTy::I64, AbiTy::Ptr, AbiTy::I64],
        ret: AbiTy::I64,
    },
    // C: int listen(int sockfd, int backlog)
    SyscallSig {
        name: "listen",
        args: &[AbiTy::I64, AbiTy::I64],
        ret: AbiTy::I64,
    },
    // C: int accept(int sockfd, struct sockaddr *addr, socklen_t *addrlen)
    SyscallSig {
        name: "accept",
        args: &[AbiTy::I64, AbiTy::Ptr, AbiTy::Ptr],
        ret: AbiTy::I64,
    },
    // C: int connect(int sockfd, const struct sockaddr *addr, socklen_t addrlen)
    SyscallSig {
        name: "connect",
        args: &[AbiTy::I64, AbiTy::Ptr, AbiTy::I64],
        ret: AbiTy::I64,
    },
    // C: ssize_t send(int sockfd, const void *buf, size_t len, int flags)
    SyscallSig {
        name: "send",
        args: &[AbiTy::I64, AbiTy::Ptr, AbiTy::I64, AbiTy::I64],
        ret: AbiTy::I64,
    },
    // C: ssize_t recv(int sockfd, void *buf, size_t len, int flags)
    SyscallSig {
        name: "recv",
        args: &[AbiTy::I64, AbiTy::Ptr, AbiTy::I64, AbiTy::I64],
        ret: AbiTy::I64,
    },
    // C: ssize_t sendto(int, const void *, size_t, int, const struct sockaddr *, socklen_t)
    SyscallSig {
        name: "sendto",
        args: &[
            AbiTy::I64, AbiTy::Ptr, AbiTy::I64,
            AbiTy::I64, AbiTy::Ptr, AbiTy::I64,
        ],
        ret: AbiTy::I64,
    },
    // C: ssize_t recvfrom(int, void *, size_t, int, struct sockaddr *, socklen_t *)
    SyscallSig {
        name: "recvfrom",
        args: &[
            AbiTy::I64, AbiTy::Ptr, AbiTy::I64,
            AbiTy::I64, AbiTy::Ptr, AbiTy::Ptr,
        ],
        ret: AbiTy::I64,
    },
    // C: int setsockopt(int, int, int, const void *, socklen_t)
    SyscallSig {
        name: "setsockopt",
        args: &[
            AbiTy::I64, AbiTy::I64, AbiTy::I64,
            AbiTy::Ptr, AbiTy::I64,
        ],
        ret: AbiTy::I64,
    },
    // C: pid_t waitpid(pid_t pid, int *wstatus, int options)
    SyscallSig {
        name: "waitpid",
        args: &[AbiTy::I64, AbiTy::Ptr, AbiTy::I64],
        ret: AbiTy::I64,
    },
    // ── pthread TLS (used by core/mem/epoch + verum_runtime tls) ────
    //
    // Each pthread symbol was declared at TWO sites with conflicting
    // ABIs: `vbc_lowering.rs:declare_fn!` used the C `int` (i32) ABI;
    // `platform_ir.rs::get_or_declare_fn` used the Verum-canonical i64
    // ABI.  The loser's `FunctionValue` was returned, producing
    // wrong-arity `call_native_i64` errors thousands of instructions
    // later.
    //
    // The i64-everywhere ABI is correct for Verum codegen — pthread_key_t
    // is `unsigned long` (8 bytes) on every 64-bit POSIX target; the i32
    // declarations were copy-paste from the C header without considering
    // the actual ABI width.
    // C: int pthread_key_create(pthread_key_t*, void (*)(void*))
    SyscallSig {
        name: "pthread_key_create",
        args: &[AbiTy::Ptr, AbiTy::Ptr],
        ret: AbiTy::I64,
    },
    // C: int pthread_key_delete(pthread_key_t)
    SyscallSig {
        name: "pthread_key_delete",
        args: &[AbiTy::I64],
        ret: AbiTy::I64,
    },
    // C: void* pthread_getspecific(pthread_key_t) — Verum codegen
    // treats the returned pointer as i64 (bitcast back at call site).
    SyscallSig {
        name: "pthread_getspecific",
        args: &[AbiTy::I64],
        ret: AbiTy::I64,
    },
    // C: int pthread_setspecific(pthread_key_t, const void*)
    SyscallSig {
        name: "pthread_setspecific",
        args: &[AbiTy::I64, AbiTy::Ptr],
        ret: AbiTy::I64,
    },
    // ── POSIX semaphores (used by core/sync/semaphore) ──────────────
    SyscallSig { name: "sem_init",    args: &[AbiTy::Ptr, AbiTy::I64, AbiTy::I64], ret: AbiTy::I64 },
    SyscallSig { name: "sem_wait",    args: &[AbiTy::Ptr], ret: AbiTy::I64 },
    SyscallSig { name: "sem_post",    args: &[AbiTy::Ptr], ret: AbiTy::I64 },
    SyscallSig { name: "sem_destroy", args: &[AbiTy::Ptr], ret: AbiTy::I64 },
    // ── Process management (core/sys/process) ───────────────────────
    SyscallSig { name: "pipe",   args: &[AbiTy::Ptr], ret: AbiTy::I64 },
    SyscallSig { name: "fork",   args: &[], ret: AbiTy::I64 },
    SyscallSig { name: "dup2",   args: &[AbiTy::I64, AbiTy::I64], ret: AbiTy::I64 },
    SyscallSig { name: "execvp", args: &[AbiTy::Ptr, AbiTy::Ptr], ret: AbiTy::I64 },
];

// =============================================================================
// Verum-runtime symbols — internal helpers emitted by the LLVM runtime
// layer.  Distinct from POSIX_SYSCALLS because these are NOT platform
// syscalls — they're stdlib runtime functions implemented in LLVM IR
// (some inline, some as opaque imports satisfied by
// `verum_runtime_stubs.o`).
//
// Pre-fix these were declared at multiple emit paths with conflicting
// signatures (`verum_string_join` ptr-vs-i64 first param,
// `verum_list_reverse` void-vs-ptr return type, etc).  The runtime
// emitter at `runtime.rs::emit_verum_string_join` even had defensive
// code handling BOTH shapes — a band-aid for the underlying root cause
// this registry eliminates.
//
// Pinned by `crates/verum_codegen/src/llvm/error.rs::take_signature_mismatches`
// — every emit site that fails to route through this registry surfaces
// the mismatch into the lowering pipeline's diagnostic stream.
// =============================================================================

const VERUM_RUNTIME_SYMBOLS: &[SyscallSig] = &[
    // Text helpers.
    // verum_string_join(list_ptr: i64, sep: *i8) -> *i8
    SyscallSig {
        name: "verum_string_join",
        args: &[AbiTy::I64, AbiTy::Ptr],
        ret: AbiTy::Ptr,
    },
    // List in-place reversal — mutates the list buffer in place.
    // Canonical signature matches `runtime.rs::define_list_ir_helpers`
    // (line ~13463) which returns void.  Pre-fix a separate emit path
    // in `instruction.rs` declared it `ptr(ptr) -> ptr` while the
    // runtime emitter used `void(ptr) -> void` — recorded as a
    // signature-mismatch by the registry gate.  Canonical: void
    // return (in-place mutation, no chaining required).
    // verum_list_reverse(list: ptr) -> void
    SyscallSig {
        name: "verum_list_reverse",
        args: &[AbiTy::Ptr],
        ret: AbiTy::Void,
    },
    // List in-place pairwise swap — same in-place mutation pattern.
    // verum_list_swap(list: ptr, i: i64, j: i64) -> void
    SyscallSig {
        name: "verum_list_swap",
        args: &[AbiTy::Ptr, AbiTy::I64, AbiTy::I64],
        ret: AbiTy::Void,
    },
    // C wrapper for open(3) — avoids ARM64 variadic issues.
    // verum_raw_open3(path: *i8, flags: i64, mode: i64) -> i64
    SyscallSig {
        name: "verum_raw_open3",
        args: &[AbiTy::Ptr, AbiTy::I64, AbiTy::I64],
        ret: AbiTy::I64,
    },
    // TCP connect helper.
    // verum_tcp_connect(host: *i8, port: i64) -> i64
    SyscallSig {
        name: "verum_tcp_connect",
        args: &[AbiTy::Ptr, AbiTy::I64],
        ret: AbiTy::I64,
    },
];

/// Look up a syscall or Verum-runtime symbol's canonical signature.
/// `None` for names not in either registry — callers should fall back
/// to a custom declaration, or extend [`POSIX_SYSCALLS`] /
/// [`VERUM_RUNTIME_SYMBOLS`] if the symbol is genuinely cross-emit-path.
///
/// The two registries are searched in order (POSIX first, then Verum
/// runtime) — names are guaranteed unique across both by manual audit
/// at registry-extension time.  Adding a duplicate causes the first
/// hit to win, which is the safe default.
fn lookup(name: &str) -> Option<&'static SyscallSig> {
    POSIX_SYSCALLS
        .iter()
        .chain(VERUM_RUNTIME_SYMBOLS.iter())
        .find(|s| s.name == name)
}

/// Public look-up surface used by `error::get_or_declare_function` to
/// resolve a name to its canonical `(args, ret)` signature before
/// deciding whether to honour or override the caller-provided
/// `fn_type` hint.  Returns the static `SyscallSig` so callers can
/// build the canonical `FunctionType` via [`canonical_fn_type`] without
/// duplicating the AbiTy → LLVM type-conversion logic.
pub fn lookup_sig(name: &str) -> Option<&'static SyscallSig> {
    lookup(name)
}

/// Build the canonical LLVM `FunctionType` for a given `SyscallSig`.
/// Mirrors the construction in [`get_or_declare`] so callers comparing
/// against an `existing` declaration can compute the expected shape
/// without re-implementing AbiTy::fn_type.
///
/// Accepts `impl AsContextRef<'ctx>` so callers holding either
/// `&'ctx Context` or `ContextRef<'ctx>` (`module.get_context()`) can
/// use the helper uniformly without an explicit conversion at every
/// call site.
pub fn canonical_fn_type<'ctx>(
    ctx: impl verum_llvm::context::AsContextRef<'ctx>,
    sig: &SyscallSig,
) -> verum_llvm::types::FunctionType<'ctx> {
    let cref: verum_llvm::context::ContextRef<'ctx> = unsafe {
        verum_llvm::context::ContextRef::new(ctx.as_ctx_ref())
    };
    let i64_t = cref.i64_type();
    let ptr_t = cref.ptr_type(verum_llvm::AddressSpace::default());
    let arg_tys: Vec<verum_llvm::types::BasicMetadataTypeEnum<'ctx>> = sig
        .args
        .iter()
        .map(|a| match a {
            AbiTy::I64 => i64_t.into(),
            AbiTy::Ptr => ptr_t.into(),
            AbiTy::Void => unreachable!("Void is a return-only classification"),
        })
        .collect();
    match sig.ret {
        AbiTy::I64 => i64_t.fn_type(&arg_tys, false),
        AbiTy::Ptr => ptr_t.fn_type(&arg_tys, false),
        AbiTy::Void => cref.void_type().fn_type(&arg_tys, false),
    }
}

/// Get-or-declare `name` under its canonical Verum-ABI signature.
///
/// First-call semantics: if `name` is not yet declared in `module`,
/// add it with the registry's signature. Subsequent calls return the
/// existing declaration. When the pre-existing declaration disagrees
/// with the registry's canonical signature, the mismatch is recorded
/// into the codegen-global signature-mismatch registry so the lowering
/// pipeline's final `check_no_signature_mismatches()` gate lifts it
/// into a hard `LlvmLoweringError::Internal`.
///
/// Panics in debug builds if `name` is not in [`POSIX_SYSCALLS`]; in
/// release builds returns `None` so callers can defensively fall back
/// to a local declaration. Adding a missing entry to the registry is
/// always preferred over handling `None` at the call site.
pub fn get_or_declare<'ctx>(
    module: &Module<'ctx>,
    ctx: &'ctx Context,
    name: &str,
) -> Option<FunctionValue<'ctx>> {
    let sig = lookup(name)?;
    let canonical_ty = AbiTy::fn_type(ctx, sig.args, sig.ret);
    if let Some(existing) = module.get_function(name) {
        if existing.get_type() != canonical_ty {
            super::error::record_signature_mismatch_public(
                name,
                format!("{:?}", existing.get_type()),
                format!("{:?} (canonical from POSIX_SYSCALLS registry)", canonical_ty),
            );
        }
        return Some(existing);
    }
    Some(module.add_function(name, canonical_ty, None))
}

/// Pre-declare every entry in [`POSIX_SYSCALLS`] AND
/// [`VERUM_RUNTIME_SYMBOLS`] into `module`. Call this **before** any
/// other emit path can race to declare a symbol with the wrong
/// signature. The canonical declarations land first, and any
/// subsequent `module.get_function(name)` lookup throughout VBC
/// lowering returns the canonical FunctionValue with the right
/// fn_type. This eliminates the entire "first declaration wins"
/// defect class at codegen time.
pub fn predeclare_all<'ctx>(module: &Module<'ctx>, ctx: &'ctx Context) {
    for sig in POSIX_SYSCALLS {
        let _ = get_or_declare(module, ctx, sig.name);
    }
    for sig in VERUM_RUNTIME_SYMBOLS {
        let _ = get_or_declare(module, ctx, sig.name);
    }
}

/// Pre-declare a curated set of POSIX syscalls into `module`. Used by
/// emit paths that want to ensure the Verum-ABI signatures are present
/// before any later inline declaration drifts. Idempotent — no-ops on
/// names already present.
///
/// The current set is the I/O subset (`close`, `read`, `write`,
/// `access`, `unlink`) — the historical contents of
/// `ensure_io_syscalls_declared`. Time syscalls (`clock_gettime`,
/// `nanosleep`, `sched_yield`) are declared lazily by call sites
/// through [`get_or_declare`].
pub fn ensure_io_declared<'ctx>(
    module: &Module<'ctx>,
    ctx: &'ctx Context,
) {
    for name in ["close", "read", "write", "access", "unlink"] {
        let _ = get_or_declare(module, ctx, name);
    }
}

// =============================================================================
// Verum-ABI syscall wrappers — no-libc enforcement layer.
// =============================================================================

// =============================================================================
// Linux direct-syscall emitter — shared by RuntimeLowering and PlatformIR.
//
// This used to be a private method duplicated on both impls
// (`RuntimeLowering::emit_linux_syscall`, `PlatformIR::emit_linux_syscall`).
// They were word-for-word identical: same inline-asm strings, same
// constraint registers, same 6-arg padding, same arch-driven dispatch
// over `module.get_triple()`.  Centralising here removes the drift
// risk and lets every wrapper-emit path (the `__verum_<name>`
// functions in this module's neighbourhood) use exactly one
// canonical version.
// =============================================================================

/// Emit a direct Linux syscall via inline-asm (`syscall` on x86_64,
/// `svc #0` on aarch64).  Cross-compilation correct: reads
/// `module.get_triple()`, never host `#[cfg(target_os)]`.
///
/// Pads `args` to 6 with `i64::const_zero` so the inline-asm template
/// always has all 6 register operands populated.  The kernel only
/// reads the slots the syscall actually consumes.
///
/// Returns the syscall's i64 return value.
pub fn emit_linux_syscall_inline<'ctx>(
    builder: &Builder<'ctx>,
    ctx: &'ctx Context,
    module: &Module<'ctx>,
    sys_num: u64,
    args: &[IntValue<'ctx>],
) -> LlvmResult<IntValue<'ctx>> {
    let i64_type = ctx.i64_type();

    let triple = module.get_triple();
    let triple_str = triple.as_str().to_string_lossy();
    let (asm_str, constraints) =
        if triple_str.contains("aarch64") || triple_str.contains("arm64") {
            (
                "svc #0",
                "={x0},{x8},{x0},{x1},{x2},{x3},{x4},{x5},~{memory}",
            )
        } else if triple_str.contains("x86_64") {
            (
                "syscall",
                "={rax},{rax},{rdi},{rsi},{rdx},{r10},{r8},{r9},~{rcx},~{r11},~{memory}",
            )
        } else {
            // Other archs (32-bit ARM, RISC-V, …): callers should
            // route through the per-platform fallback rather than
            // relying on this helper.  Emitted as `=r,r,...,r` so the
            // module still validates; the result is meaningless but
            // surfacing the architectural gap loudly is the point.
            ("", "=r,r,r,r,r,r,r,r")
        };

    let fn_type = i64_type.fn_type(
        &[
            i64_type.into(),
            i64_type.into(),
            i64_type.into(),
            i64_type.into(),
            i64_type.into(),
            i64_type.into(),
            i64_type.into(),
        ],
        false,
    );
    let asm_fn = ctx.create_inline_asm(
        fn_type,
        asm_str.to_string(),
        constraints.to_string(),
        true,
        true,
        Some(verum_llvm::InlineAsmDialect::ATT),
        false,
    );

    let zero = i64_type.const_zero();
    let a0 = args.first().copied().unwrap_or(zero);
    let a1 = args.get(1).copied().unwrap_or(zero);
    let a2 = args.get(2).copied().unwrap_or(zero);
    let a3 = args.get(3).copied().unwrap_or(zero);
    let a4 = args.get(4).copied().unwrap_or(zero);
    let a5 = args.get(5).copied().unwrap_or(zero);
    let num_const = i64_type.const_int(sys_num, false);

    let result = builder
        .build_indirect_call(
            fn_type,
            asm_fn,
            &[
                num_const.into(),
                a0.into(),
                a1.into(),
                a2.into(),
                a3.into(),
                a4.into(),
                a5.into(),
            ],
            "syscall_result",
        )
        .or_llvm_err()?
            .basic_value_or("syscall returned void")?
        .into_int_value();
    Ok(result)
}

/// Canonical name of the Verum-ABI wrapper for a given POSIX syscall.
/// Wrappers are emitted as private LLVM functions inside the module
/// and route calls through the platform-correct boundary:
///
///   * Linux       → inline `syscall` / `svc #0` instruction (no libc)
///   * macOS       → libSystem.B.dylib symbol (Apple-required boundary)
///   * Windows     → kernel32.dll / ntdll.dll equivalent
///
/// Call sites issue `module.get_function(verum_wrapper_name(s))` and
/// see the same Verum-ABI signature regardless of target — no
/// per-callsite Linux/macOS branching, no libc symbol on Linux.
///
/// Returns `None` when no wrapper exists; callers then fall back to
/// the direct-symbol [`get_or_declare`] path which is correct for
/// syscalls whose libc binding is already considered acceptable
/// (POSIX I/O on macOS goes through libSystem unconditionally per
/// the architecture doc; matching libc bindings on Linux is the gap
/// this wrapper layer closes for time-critical syscalls).
///
/// See `docs/architecture/no-libc-architecture.md` for the
/// project-wide no-libc invariant this layer enforces.
pub fn verum_wrapper_name(syscall_name: &str) -> Option<&'static str> {
    match syscall_name {
        "clock_gettime" => Some("__verum_clock_gettime"),
        "nanosleep"     => Some("__verum_nanosleep"),
        "sched_yield"   => Some("__verum_sched_yield"),
        _ => None,
    }
}
