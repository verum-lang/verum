//! # Industrial-Grade Intrinsic System
//!

//! This module implements a zero-overhead intrinsic system for Verum that:
//!

//! - Maps intrinsic names to optimal VBC instruction sequences
//! - Provides compile-time constant folding for pure intrinsics
//! - Enables MLIR/LLVM lowering with full optimization pass compatibility
//! - Maximizes interpreter performance through direct dispatch
//!

//! ## Design Principles
//!

//! 1. **Zero Overhead**: Intrinsics compile to inline VBC opcodes, not function calls
//! 2. **LLVM Transparent**: MLIR lowering produces operations LLVM can fully optimize
//! 3. **Interpreter Fast Path**: Hot intrinsics use dedicated dispatch handlers
//! 4. **Compile-Time Evaluation**: Pure intrinsics fold to constants when possible
//!

//! ## Intrinsic Categories
//!

//! | Category | VBC Mapping | Example |
//! |----------|-------------|---------|
//! | Arithmetic | Direct opcode | `add_i64` → AddI |
//! | Math | Inline sequence | `sqrt_f64` → SqrtF |
//! | Atomic | V-LLSI opcode | `atomic_load_u64` → AtomicLoad |
//! | Memory | Inline sequence | `memcpy` → optimized copy loop |
//! | System | Syscall opcode | `syscall0` → SyscallLinux |
//!

//! ## Performance Targets
//!

//! | Operation | VBC Interpreter | AOT/LLVM |
//! |-----------|-----------------|----------|
//! | add_i64 | 1 cycle | 1 cycle |
//! | sqrt_f64 | 15 cycles | 3 cycles |
//! | atomic_load | 5 cycles | 3 cycles |
//! | memcpy(64) | 20 cycles | 8 cycles |

pub mod codegen;
pub mod lowering;
pub mod registry;
pub mod signatures;

pub use codegen::IntrinsicCodegen;
pub use lowering::IntrinsicLowering;
pub use registry::{
    INTRINSIC_REGISTRY, Intrinsic, IntrinsicCategory, IntrinsicHint, IntrinsicRegistry,
    IntrinsicResult,
};
pub use signatures::{
    INTRINSIC_SIGNATURES, IntrinsicSignature, IntrinsicType, ProtocolBound, SignatureError,
    TypeParam, get_signature,
};

use crate::instruction::Opcode;

/// Result of intrinsic lookup.
#[derive(Debug, Clone)]
pub struct IntrinsicInfo {
    /// The intrinsic definition.
    pub intrinsic: &'static Intrinsic,
    /// Primary VBC opcode (if direct mapping exists).
    pub primary_opcode: Option<Opcode>,
    /// Whether this intrinsic is pure (no side effects).
    pub is_pure: bool,
    /// Whether this intrinsic can be evaluated at compile time.
    pub is_const_eval: bool,
}

/// Lookup an intrinsic by name.
///

/// Returns None if the intrinsic is not registered.
#[inline]
pub fn lookup_intrinsic(name: &str) -> Option<IntrinsicInfo> {
    // Try direct lookup first, then resolve LLVM-style aliases from @intrinsic declarations
    let resolved = INTRINSIC_REGISTRY.lookup(name).or_else(|| {
        let alias = match name {
            // Type conversion intrinsics
            "sitofp" | "uitofp" => "int_to_float",
            "fptosi" | "fptoui" => "float_to_int",
            "fpext" => "fpext",
            "fptrunc" => "fptrunc",
            "sext" => "sext",
            "zext" => "zext",
            "trunc" => "int_trunc",
            "bitcast" => "bitcast",
            "to_le_bytes" | "to_ne_bytes" => "to_le_bytes",
            "to_be_bytes" => "to_be_bytes",
            "from_le_bytes" | "from_ne_bytes" => "from_le_bytes",
            "from_be_bytes" => "from_be_bytes",
            "to_le" | "from_le" => "to_le_bytes",
            "to_be" | "from_be" => "to_be_bytes",
            // Float math intrinsics (generic → f64 default)
            "sqrt" => "sqrt_f64",
            "cbrt" => "cbrt_f64",
            "exp" => "exp_f64",
            "exp2" => "exp2_f64",
            "expm1" => "expm1_f64",
            "log" => "log_f64",
            "log2" => "log2_f64",
            "log10" => "log10_f64",
            "log1p" => "log1p_f64",
            "pow" => "pow_f64",
            "powi" => "powi_f64",
            "sin" => "sin_f64",
            "cos" => "cos_f64",
            "tan" => "tan_f64",
            "asin" => "asin_f64",
            "acos" => "acos_f64",
            "atan" => "atan_f64",
            "atan2" => "atan2_f64",
            "sinh" => "sinh_f64",
            "cosh" => "cosh_f64",
            "tanh" => "tanh_f64",
            "asinh" => "asinh_f64",
            "acosh" => "acosh_f64",
            "atanh" => "atanh_f64",
            "hypot" => "hypot_f64",
            "fma" => "fma_f64",
            "floor" => "floor_f64",
            "ceil" => "ceil_f64",
            "round" => "round_f64",
            "fabs" => "abs_f64",
            "abs" => "abs_signed",
            "copysign" => "copysign_f64",
            "fmod" => "fmod_f64",
            "minnum" => "minnum_f64",
            "maxnum" => "maxnum_f64",
            // LLVM-style intrinsic names (from @intrinsic("llvm.xxx.f64") declarations)
            "llvm.sqrt.f64" => "sqrt_f64",
            "llvm.sqrt.f32" => "sqrt_f32",
            "llvm.sin.f64" => "sin_f64",
            "llvm.sin.f32" => "sin_f32",
            "llvm.cos.f64" => "cos_f64",
            "llvm.cos.f32" => "cos_f32",
            "llvm.exp.f64" => "exp_f64",
            "llvm.exp.f32" => "exp_f32",
            "llvm.exp2.f64" => "exp2_f64",
            "llvm.exp2.f32" => "exp2_f32",
            "llvm.log.f64" => "log_f64",
            "llvm.log.f32" => "log_f32",
            "llvm.log2.f64" => "log2_f64",
            "llvm.log2.f32" => "log2_f32",
            "llvm.log10.f64" => "log10_f64",
            "llvm.log10.f32" => "log10_f32",
            "llvm.pow.f64" => "pow_f64",
            "llvm.pow.f32" => "pow_f32",
            "llvm.powi.f64" => "powi_f64",
            "llvm.powi.f32" => "powi_f32",
            "llvm.floor.f64" => "floor_f64",
            "llvm.floor.f32" => "floor_f32",
            "llvm.ceil.f64" => "ceil_f64",
            "llvm.ceil.f32" => "ceil_f32",
            "llvm.round.f64" => "round_f64",
            "llvm.round.f32" => "round_f32",
            "llvm.trunc.f64" => "trunc_f64",
            "llvm.trunc.f32" => "trunc_f32",
            "llvm.fabs.f64" => "abs_f64",
            "llvm.fabs.f32" => "abs_f32",
            "llvm.copysign.f64" => "copysign_f64",
            "llvm.copysign.f32" => "copysign_f32",
            "llvm.fma.f64" => "fma_f64",
            "llvm.fma.f32" => "fma_f32",
            "llvm.minnum.f64" => "minnum_f64",
            "llvm.minnum.f32" => "minnum_f32",
            "llvm.maxnum.f64" => "maxnum_f64",
            "llvm.maxnum.f32" => "maxnum_f32",
            // Float classification intrinsics
            "is_nan" => "is_nan_f64",
            "is_inf" | "is_infinite" => "is_infinite_f64",
            "is_finite" => "is_finite_f64",
            // is_normal doesn't have a dedicated entry - use is_finite as approximation
            "is_normal" => "is_finite_f64",
            // Float special values (these exist in registry)
            "f64_infinity" | "infinity" => "f64_infinity",
            "f64_neg_infinity" => "f64_neg_infinity",
            "f64_nan" | "nan" => "f64_nan",
            // Generic atomic intrinsics — `core/intrinsics/atomic.vr`
            // declares `atomic_load<T>(...)` / `atomic_store<T>(...)`
            // without a width suffix.  The Tier-0 / Tier-1 lowering
            // operates at machine-word width, so the bare name is an
            // alias of the canonical 64-bit form.  The dispatch
            // honours MemoryOrder via the ordering operand (drops to
            // SeqCst for Tier-0; LLVM consumes the operand directly
            // for Tier-1).  Same alias-rule shape as the float-math
            // entries above.
            "atomic_load" => "atomic_load_u64",
            "atomic_store" => "atomic_store_u64",
            "atomicrmw_xchg" => "atomic_exchange_u64",
            "atomicrmw_add" => "atomic_fetch_add_u64",
            "atomicrmw_sub" => "atomic_fetch_sub_u64",
            "atomicrmw_and" => "atomic_fetch_and_u64",
            "atomicrmw_or" => "atomic_fetch_or_u64",
            "atomicrmw_xor" => "atomic_fetch_xor_u64",
            "cmpxchg" | "cmpxchg_weak" => "atomic_cas_u64",
            // LLVM-canonical names for bit-manipulation intrinsics.
            //
            // `core/intrinsics/bitwise.vr` declares the public surface
            // (`clz<T>`, `clz_u32`, `clz_u64`, etc.) with bodies that call
            // `@intrinsic("ctlz", x)` — the LLVM-style name.  Without these
            // aliases, the lookup fails and `compile_intrinsic_call` emits
            // `LoadNil`, silently returning `nil` to the caller.  The
            // resulting bytecode then propagates the nil through the rest
            // of the calling expression, producing nonsense bit-arithmetic
            // (e.g. `clz_u64(1791) = nil` → `63 - 0 = 63` → wrong bsr →
            // `size_to_bin_large` mis-classifies every allocation above
            // 1024 bytes).  Discovered via
            // `core-tests/mem/size_class/property_test::law_round_trip_full_table_exhaustive`.
            //
            // Width is irrelevant for these inline-sequence intrinsics —
            // the interpreter dispatches `ArithSubOpcode::Clz / Ctz /
            // Popcnt / Bswap / BitReverse` against the 64-bit canonical
            // form regardless of the caller's declared parameter width.
            // Callers that need narrower-width semantics either bit-mask
            // before / after, or — for the typed-suffix surface (clz_u32,
            // popcnt_u32, etc.) — route through their own registry
            // entries above (lines 2948-2998).
            "ctlz" => "clz",
            "cttz" => "ctz",
            "ctpop" => "popcnt",
            // LLVM canonical i64/i32 spellings — `core/math/bits.vr`
            // declares bodyless wrappers tagged
            // `@intrinsic("llvm.ctlz.i64")` / `llvm.cttz.i64` /
            // `llvm.ctpop.i64` / `llvm.bswap.i64` (and i32 siblings).
            // Without these aliases, the user-side `compile_call`
            // intercept at `expressions.rs:4592` resolved the
            // intrinsic-name lookup to None and fell through to a
            // raw `Call` to the bodyless function → executed an
            // empty body → returned `Value::default() = Unit`,
            // surfacing as `clz(1 as UInt64) = ()` instead of 63.
            // The sister `popcnt` only worked because
            // `core/base/primitives.vr` provides a LOCAL bodied
            // wrapper `fn popcnt<T>(x: T) -> T { @intrinsic("ctpop",
            // x) }` whose `"ctpop"` alias was already wired above —
            // an asymmetry between `mount`-imported vs locally-
            // defined intrinsic wrappers that the symmetric alias
            // map below now closes.  Task #25 [E3] CLOSED.
            "llvm.ctlz.i64" | "llvm.ctlz.i32" => "clz",
            "llvm.cttz.i64" | "llvm.cttz.i32" => "ctz",
            "llvm.ctpop.i64" | "llvm.ctpop.i32" => "popcnt",
            "llvm.bswap.i64" | "llvm.bswap.i32" | "llvm.bswap.i16" => "bswap",
            "llvm.bitreverse.i64" | "llvm.bitreverse.i32" => "bitreverse",
            // Funnel shifts — `fshl(a, b, c)` and `fshr(a, b, c)` are
            // 3-operand: concatenate `a:b` into a 128-bit value, shift
            // left/right by `c`, return the appropriate half.
            //
            // Pre-fix this dispatched through `rotate_left` / `rotate_right`
            // (2-operand) — silently wrong because emit_arith_extended_binary
            // takes args[0]/args[1] only, DROPPING the third argument.
            // For `rotl<T>(x, n) = @intrinsic("fshl", x, x, n)` callers
            // this meant the amount-of-rotation was passed AS the
            // rotation-amount-input (args[1] = x, args[2] = n was dropped).
            //
            // The fix is twofold:
            //   1. The Tier-0 interpreter gained a dedicated `Fshl` /
            //      `Fshr` ArithSubOpcode (3-operand `hi, lo, amount`).
            //   2. `core/intrinsics/bitwise.vr::rotl` / `rotr` rewritten
            //      to use the canonical 2-arg `@intrinsic("rotate_left",
            //      x, n)` directly — avoiding the funnel-shift detour
            //      for the degenerate case.
            //   3. `core/intrinsics/bitwise.vr::fshl<T>(a, b, c)` and
            //      `fshr` route to the new 3-operand opcode via
            //      `InlineSequenceId::Fshl` / `Fshr` registered below.
            //
            // The aliases below stay for backward-compat in case any
            // external caller used the LLVM-canonical names, now
            // routing through the CORRECT 3-operand registry entries.
            "fshl" => "fshl_u64",
            "fshr" => "fshr_u64",
            // LLVM-canonical 3-arg funnel-shift width variants — closes
            // the bodyless-decl alias gap so any future
            // `@intrinsic("llvm.fshl.iW")` annotation on a 3-arg fn
            // resolves to the dedicated `fshl_u64` / `fshr_u64` registry
            // entry (3-operand `ArithSubOpcode::FunnelShift{Left,Right}`,
            // 0x57 / 0x58 — the correct opcodes that preserve all three
            // operands).  Arity-mismatched 2-arg declarations should
            // instead use `rotate_left` / `rotate_right` per the rule
            // pinned in `core/math/bits.vr::rotl`.
            "llvm.fshl.i64" | "llvm.fshl.i32" | "llvm.fshl.i16" => "fshl_u64",
            "llvm.fshr.i64" | "llvm.fshr.i32" | "llvm.fshr.i16" => "fshr_u64",
            _ => {
                // Strip common prefixes used in import aliases
                // e.g., intrinsic_memcpy → memcpy, intrinsic_slice_from_raw_parts_mut → slice_from_raw_parts_mut
                if let Some(stripped) = name.strip_prefix("intrinsic_")
                    && let Some(found) = INTRINSIC_REGISTRY.lookup(stripped)
                {
                    return Some(found);
                }
                // Generic fallback: try UPPER_CASE version of the name
                // This handles tensor_new → TENSOR_NEW, gpu_malloc → GPU_MALLOC, etc.
                let upper = name.to_uppercase();
                return INTRINSIC_REGISTRY.lookup(&upper);
            }
        };
        INTRINSIC_REGISTRY.lookup(alias)
    });
    resolved.map(|intrinsic| IntrinsicInfo {
        intrinsic,
        primary_opcode: intrinsic.primary_opcode(),
        is_pure: intrinsic.hints.contains(&IntrinsicHint::Pure),
        is_const_eval: intrinsic.hints.contains(&IntrinsicHint::ConstEval),
    })
}

/// Check if a name is a registered intrinsic.
#[inline]
pub fn is_intrinsic(name: &str) -> bool {
    INTRINSIC_REGISTRY.contains(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intrinsic_lookup() {
        let info = lookup_intrinsic("add_i64").unwrap();
        assert_eq!(info.primary_opcode, Some(Opcode::AddI));
        assert!(info.is_pure);
    }

    #[test]
    fn test_atomic_intrinsic() {
        let info = lookup_intrinsic("atomic_load_u64").unwrap();
        assert_eq!(info.primary_opcode, Some(Opcode::AtomicLoad));
        assert!(!info.is_pure); // Atomics have side effects
    }

    #[test]
    fn test_unknown_intrinsic() {
        assert!(lookup_intrinsic("nonexistent").is_none());
    }
}
