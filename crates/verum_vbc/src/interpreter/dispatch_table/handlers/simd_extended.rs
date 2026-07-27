//! SIMD extended opcode handler for VBC interpreter dispatch.

use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;
use super::envelope::dispatch_enveloped;
use crate::instruction::{Opcode, SimdSubOpcode};
use crate::value::Value;

/// SimdExtended (0x2A) - Platform-agnostic SIMD operations.
///
/// Format: `[0x2A] [sub_opcode:u8] [operands...]`
///
/// Sub-opcode categories:
/// - 0x00-0x0F: Vector Creation (Splat, Extract, Insert, FromScalars)
/// - 0x10-0x1F: Arithmetic (Add, Sub, Mul, Div, Neg, Abs, Sqrt, Fma, Min, Max)
/// - 0x30-0x3F: Reductions (ReduceAdd, ReduceMul, ReduceMin, ReduceMax)
/// - 0x40-0x4F: Comparisons (CmpEq, CmpNe, CmpLt, CmpLe, CmpGt, CmpGe, Select)
/// - 0x50-0x5F: Memory (LoadAligned, StoreAligned, Gather, Scatter)
/// - 0x60-0x6F: Shuffle/Permute (Shuffle, Permute, Reverse, Rotate)
/// - 0x70-0x7F: Bitwise (BitwiseAnd, BitwiseOr, BitwiseXor, BitwiseNot, Shifts)
/// - 0x80-0x8F: Mask Operations (MaskAll, MaskNone, MaskAny)
/// - 0x90-0x9F: Type Conversion (Cast, Convert*)
///
/// Note: SIMD operations require platform-specific support. BOTH tiers
/// implement the same scalar fallback — a "vector" register carries one lane
/// — because the wire erases the element type and lane count of the
/// source-level `Vec<T, N>`. AOT does NOT emit LLVM vector intrinsics: the
/// typed `SimdLowering` API in `verum_codegen`'s `llvm/simd.rs` has no
/// callers, and `lower_simd_extended` mirrors these arms one for one.
/// The MEMORY family is the exception both tiers share — see
/// [`simd_memory_op_unimplemented`].
pub(in super::super) fn handle_simd_extended(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    dispatch_enveloped(state, simd_extended_body)
}

/// The SIMD MEMORY family — the four stores `StoreAligned` / `StoreUnaligned`
/// / `MaskedStore` / `Scatter` (T0112) and the four loads `LoadAligned` /
/// `LoadUnaligned` / `MaskedLoad` / `Gather` (T0184) — has no honest scalar
/// fallback, so it refuses loudly rather than inventing an answer.
///
/// Both halves were silently wrong, in the two different ways this class
/// takes:
///
/// * every STORE arm read its operands and returned
///   `Ok(DispatchResult::Continue)`.  The store ran, the destination was never
///   touched, and the caller saw success.  A dropped write is worse than a
///   wrong value — the destination keeps STALE DATA, which is neither uniform
///   nor implausible, so nothing downstream can detect it.  It hit the
///   ORDINARY aligned store, not only the exotic masked/scatter forms.
/// * every LOAD arm answered `dst = ptr` — the ADDRESS ITSELF as the loaded
///   value, never a dereference.  That is fabricated data: a plausible number
///   that flows onward indistinguishable from a real lane.
///
/// Neither has a correct scalar form, for one shared reason.  The wire is
/// `[dst][…]` — `encode_operands` prefixes the destination register
/// unconditionally, regardless of `return_count` — with both the element type
/// `T` and the lane count `N` erased.  A store of the one register value would
/// leave `N - 1` lanes stale AND, for any `T` narrower than the 8-byte
/// register, clobber the neighbouring element; a load has no width to read and
/// no way to fill lanes `1..N`.  Refusing is the only answer that neither
/// corrupts memory nor fabricates a value.
///
/// The AOT twin (`lower_simd_extended` in `verum_codegen`) aborts on exactly
/// these eight sub-ops, so the tiers stay coherent.  The neighbouring
/// shuffle / cast / mask arms KEEP their scalar fallbacks: those are honest at
/// width 1, because a one-lane shuffle really is the identity.  It is the
/// memory ops that have no correct form.
///
/// ONE constructor so every memory op reports identically, naming the sub-op
/// that refused.
fn simd_memory_op_unimplemented(feature: &'static str) -> InterpreterError {
    InterpreterError::NotImplemented {
        feature,
        opcode: Some(Opcode::SimdExtended),
    }
}

/// `SimdExtended` sub-op arms. Invoked through
/// [`dispatch_enveloped`](super::envelope::dispatch_enveloped), which owns the
/// sub-op byte, the operand-length envelope and the pc reposition — an arm may
/// read any number of operands, and may `return` early, without desynchronising
/// the instruction stream.
fn simd_extended_body(
    state: &mut InterpreterState,
    sub_op_byte: u8,
) -> InterpreterResult<DispatchResult> {
    let sub_op = SimdSubOpcode::from_byte(sub_op_byte);

    match sub_op {
        // ================================================================
        // Vector Creation (0x00-0x0F)
        // ================================================================
        Some(SimdSubOpcode::Splat) => {
            // Splat scalar to vector: dst[all lanes] = src
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            // In interpreter mode, store as single value (scalar fallback)
            let val = state.get_reg(src_reg);
            state.set_reg(dst, val);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Extract) => {
            // Extract single lane from vector
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            // The lane index is a REGISTER on the wire, not an immediate:
            // `encode_operands` packs every intrinsic argument as a register,
            // and `simd_extract(vec, idx)` passes `idx` as an ordinary
            // argument. Reading it with `read_u8` consumed one byte and
            // treated the register NUMBER as the lane, which also mis-sized
            // the operand for any register >= 128 (two-byte encoding).
            let _lane_reg = read_reg(state)?;
            // Scalar fallback: just return the value
            let val = state.get_reg(src_reg);
            state.set_reg(dst, val);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Insert) => {
            // Insert into single lane
            let dst = read_reg(state)?;
            let vec_reg = read_reg(state)?;
            // The lane index is a REGISTER on the wire, not an immediate:
            // `encode_operands` packs every intrinsic argument as a register,
            // and `simd_extract(vec, idx)` passes `idx` as an ordinary
            // argument. Reading it with `read_u8` consumed one byte and
            // treated the register NUMBER as the lane, which also mis-sized
            // the operand for any register >= 128 (two-byte encoding).
            let _lane_reg = read_reg(state)?;
            let val_reg = read_reg(state)?;
            // Scalar fallback: use the inserted value
            let _ = state.get_reg(vec_reg);
            let val = state.get_reg(val_reg);
            state.set_reg(dst, val);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::FromScalars) => {
            // Create vector from scalars (uses RegRange)
            let dst = read_reg(state)?;
            let range = read_reg_range(state)?;
            // Scalar fallback: use first element
            let first = state.get_reg(range.start);
            state.set_reg(dst, first);
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Arithmetic (0x10-0x1F) - Scalar fallback implementations
        // ================================================================
        Some(SimdSubOpcode::Add) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_f64();
            let b = state.get_reg(b_reg).as_f64();
            state.set_reg(dst, Value::from_f64(a + b));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Sub) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_f64();
            let b = state.get_reg(b_reg).as_f64();
            state.set_reg(dst, Value::from_f64(a - b));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Mul) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_f64();
            let b = state.get_reg(b_reg).as_f64();
            state.set_reg(dst, Value::from_f64(a * b));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Div) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_f64();
            let b = state.get_reg(b_reg).as_f64();
            state.set_reg(dst, Value::from_f64(a / b));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Neg) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let x = state.get_reg(src_reg).as_f64();
            state.set_reg(dst, Value::from_f64(-x));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Abs) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let x = state.get_reg(src_reg).as_f64();
            state.set_reg(dst, Value::from_f64(x.abs()));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Sqrt) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let x = state.get_reg(src_reg).as_f64();
            state.set_reg(dst, Value::from_f64(x.sqrt()));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Fma) => {
            // Fused multiply-add: dst = a * b + c
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let c_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_f64();
            let b = state.get_reg(b_reg).as_f64();
            let c = state.get_reg(c_reg).as_f64();
            state.set_reg(dst, Value::from_f64(a.mul_add(b, c)));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Min) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_f64();
            let b = state.get_reg(b_reg).as_f64();
            state.set_reg(dst, Value::from_f64(a.min(b)));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Max) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_f64();
            let b = state.get_reg(b_reg).as_f64();
            state.set_reg(dst, Value::from_f64(a.max(b)));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Rem) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_f64();
            let b = state.get_reg(b_reg).as_f64();
            state.set_reg(dst, Value::from_f64(a % b));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Recip) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let x = state.get_reg(src_reg).as_f64();
            state.set_reg(dst, Value::from_f64(1.0 / x));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Rsqrt) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let x = state.get_reg(src_reg).as_f64();
            state.set_reg(dst, Value::from_f64(1.0 / x.sqrt()));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Reductions (0x30-0x3F)
        // ================================================================
        Some(SimdSubOpcode::ReduceAdd) => {
            // Horizontal add reduction (scalar fallback returns the value)
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let x = state.get_reg(src_reg);
            state.set_reg(dst, x);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::ReduceMul) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let x = state.get_reg(src_reg);
            state.set_reg(dst, x);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::ReduceMin) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let x = state.get_reg(src_reg);
            state.set_reg(dst, x);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::ReduceMax) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let x = state.get_reg(src_reg);
            state.set_reg(dst, x);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::ReduceAnd) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let x = state.get_reg(src_reg);
            state.set_reg(dst, x);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::ReduceOr) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let x = state.get_reg(src_reg);
            state.set_reg(dst, x);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::ReduceXor) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let x = state.get_reg(src_reg);
            state.set_reg(dst, x);
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Comparisons (0x40-0x4F)
        // ================================================================
        Some(SimdSubOpcode::CmpEq) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_f64();
            let b = state.get_reg(b_reg).as_f64();
            state.set_reg(dst, Value::from_bool(a == b));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::CmpNe) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_f64();
            let b = state.get_reg(b_reg).as_f64();
            state.set_reg(dst, Value::from_bool(a != b));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::CmpLt) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_f64();
            let b = state.get_reg(b_reg).as_f64();
            state.set_reg(dst, Value::from_bool(a < b));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::CmpLe) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_f64();
            let b = state.get_reg(b_reg).as_f64();
            state.set_reg(dst, Value::from_bool(a <= b));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::CmpGt) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_f64();
            let b = state.get_reg(b_reg).as_f64();
            state.set_reg(dst, Value::from_bool(a > b));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::CmpGe) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_f64();
            let b = state.get_reg(b_reg).as_f64();
            state.set_reg(dst, Value::from_bool(a >= b));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Select) => {
            // Select/blend based on mask
            let dst = read_reg(state)?;
            let mask_reg = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let mask = state.get_reg(mask_reg).as_bool();
            let a = state.get_reg(a_reg);
            let b = state.get_reg(b_reg);
            state.set_reg(dst, if mask { a } else { b });
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Memory Operations (0x50-0x5F)
        //
        // The WHOLE memory family refuses via
        // [`simd_memory_op_unimplemented`] — see that function for why neither
        // a scalar load nor a scalar store is correct. These arms deliberately
        // read NO operands: `dispatch_enveloped` owns the pc reposition and
        // applies it on the `Err` path too, so there is nothing to consume for
        // alignment.
        // ================================================================
        Some(SimdSubOpcode::LoadAligned) => Err(simd_memory_op_unimplemented("simd_load_aligned")),
        Some(SimdSubOpcode::LoadUnaligned) => {
            Err(simd_memory_op_unimplemented("simd_load_unaligned"))
        }

        Some(SimdSubOpcode::StoreAligned) => Err(simd_memory_op_unimplemented("simd_store_aligned")),
        Some(SimdSubOpcode::StoreUnaligned) => {
            Err(simd_memory_op_unimplemented("simd_store_unaligned"))
        }

        Some(SimdSubOpcode::MaskedLoad) => Err(simd_memory_op_unimplemented("simd_masked_load")),

        Some(SimdSubOpcode::MaskedStore) => Err(simd_memory_op_unimplemented("simd_masked_store")),

        Some(SimdSubOpcode::Gather) => Err(simd_memory_op_unimplemented("simd_gather")),

        Some(SimdSubOpcode::Scatter) => Err(simd_memory_op_unimplemented("simd_scatter")),

        // ================================================================
        // Shuffle/Permute (0x60-0x6F)
        // ================================================================
        Some(SimdSubOpcode::Shuffle) => {
            // Arity is call-site dependent: the method form
            // `shuffle<MASK: meta>(self, other)` packs 3 registers while the free
            // function `simd_shuffle(a, b, mask)` packs 4. The scalar fallback
            // returns `a` either way, so a trailing indices operand is left
            // unread — safe because the envelope, not this arm, decides where
            // the next instruction starts.
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let _b_reg = read_reg(state)?;
            // Scalar fallback: return first operand
            let a = state.get_reg(a_reg);
            state.set_reg(dst, a);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Permute) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let _control_reg = read_reg(state)?;
            let src = state.get_reg(src_reg);
            state.set_reg(dst, src);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Reverse) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let src = state.get_reg(src_reg);
            state.set_reg(dst, src);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Rotate) => {
            // `rotate_left<COUNT: meta USize>(self)` resolves COUNT at compile
            // time, so the wire is [dst][self] with no rotate-amount operand.
            // The `read_u8` here consumed a byte that was never emitted.
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let src = state.get_reg(src_reg);
            state.set_reg(dst, src);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::InterleaveLow) | Some(SimdSubOpcode::InterleaveHigh) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let _b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg);
            state.set_reg(dst, a);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Concat) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let _b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg);
            state.set_reg(dst, a);
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Bitwise Operations (0x70-0x7F)
        // ================================================================
        Some(SimdSubOpcode::BitwiseAnd) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64();
            state.set_reg(dst, Value::from_i64(a & b));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::BitwiseOr) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64();
            state.set_reg(dst, Value::from_i64(a | b));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::BitwiseXor) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64();
            state.set_reg(dst, Value::from_i64(a ^ b));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::BitwiseNot) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let x = state.get_reg(src_reg).as_i64();
            state.set_reg(dst, Value::from_i64(!x));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::ShiftLeft) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let shift_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_i64();
            let shift = state.get_reg(shift_reg).as_i64() as u32;
            state.set_reg(dst, Value::from_i64(a << shift));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::ShiftRight) | Some(SimdSubOpcode::ShiftRightArith) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let shift_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_i64();
            let shift = state.get_reg(shift_reg).as_i64() as u32;
            state.set_reg(dst, Value::from_i64(a >> shift));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::AndNot) => {
            // a & ~b
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64();
            state.set_reg(dst, Value::from_i64(a & !b));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Mask Operations (0x80-0x8F)
        // ================================================================
        Some(SimdSubOpcode::MaskAll) => {
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_bool(true));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::MaskNone) => {
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_bool(false));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::MaskAny) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let x = state.get_reg(src_reg).as_bool();
            state.set_reg(dst, Value::from_bool(x));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::MaskCount) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            // Scalar fallback: 1 if true, 0 if false
            let x = state.get_reg(src_reg).as_bool();
            state.set_reg(dst, Value::from_i64(if x { 1 } else { 0 }));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::MaskFirstTrue) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            // Scalar fallback: 0 if true, -1 if false (no true lane)
            let x = state.get_reg(src_reg).as_bool();
            state.set_reg(dst, Value::from_i64(if x { 0 } else { -1 }));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Compress) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let _mask_reg = read_reg(state)?;
            let src = state.get_reg(src_reg);
            state.set_reg(dst, src);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Expand) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let _mask_reg = read_reg(state)?;
            let src = state.get_reg(src_reg);
            state.set_reg(dst, src);
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Type Conversion (0x90-0x9F)
        // ================================================================
        Some(SimdSubOpcode::Cast) => {
            // Generic type cast (scalar fallback is identity)
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let x = state.get_reg(src_reg);
            state.set_reg(dst, x);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::ConvertF32ToF64) | Some(SimdSubOpcode::ConvertF64ToF32) => {
            // Width conversion (scalar fallback is identity for f64)
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let x = state.get_reg(src_reg);
            state.set_reg(dst, x);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::ConvertIntToFloat) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let x = state.get_reg(src_reg).as_i64();
            state.set_reg(dst, Value::from_f64(x as f64));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::ConvertFloatToInt) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let x = state.get_reg(src_reg).as_f64();
            state.set_reg(dst, Value::from_i64(x as i64));
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Bitcast) => {
            // Reinterpret bits (scalar fallback is identity)
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let x = state.get_reg(src_reg);
            state.set_reg(dst, x);
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Unimplemented sub-opcodes
        // ================================================================
        None => Err(InterpreterError::NotImplemented {
            feature: "simd_extended sub-opcode",
            opcode: Some(Opcode::SimdExtended),
        }),
    }
}

#[cfg(test)]
mod simd_store_never_silently_drops_tests {
    use super::*;
    use crate::bytecode;
    use crate::instruction::{Instruction, Reg};
    use crate::interpreter::Interpreter;
    use crate::module::{FunctionDescriptor, FunctionId, VbcModule};
    use crate::types::StringId;
    use std::sync::Arc;

    /// Value returned by the instruction AFTER the carrier. Reaching it is
    /// exactly the pre-fix failure mode: the program sailed past a store that
    /// wrote nothing and reported success.
    const SENTINEL: i8 = 41;

    fn run(instructions: &[Instruction]) -> InterpreterResult<Value> {
        let mut bc = Vec::new();
        for instr in instructions {
            bytecode::encode_instruction(instr, &mut bc);
        }
        let mut module = VbcModule::new("simd_store_pins".to_string());
        let mut func = FunctionDescriptor::new(StringId::EMPTY);
        func.id = FunctionId(0);
        func.bytecode_offset = 0;
        func.bytecode_length = bc.len() as u32;
        func.register_count = 16;
        module.functions.push(func);
        module.bytecode = bc;
        Interpreter::new(Arc::new(module)).execute_function(FunctionId(0))
    }

    /// Run `carrier`, then load SENTINEL and return it. A store that drops the
    /// write silently produces `Ok(SENTINEL)`; a store that refuses produces
    /// `Err`.
    fn run_carrier_then_sentinel(carrier: Instruction) -> InterpreterResult<Value> {
        run(&[
            carrier,
            Instruction::LoadSmallI {
                dst: Reg(0),
                value: SENTINEL,
            },
            Instruction::Ret { value: Reg(0) },
        ])
    }

    fn store_carrier(sub_op: SimdSubOpcode) -> Instruction {
        // Wire is `[dst][src][ptr](…)` — `encode_operands` prefixes the
        // destination register unconditionally, even for a void sub-op. The
        // trailing mask/indices operand of MaskedStore/Scatter is included so
        // the envelope length matches a real emission.
        Instruction::SimdExtended {
            sub_op: sub_op as u8,
            operands: vec![1, 2, 3, 4],
        }
    }

    /// Every SIMD store sub-op refuses. Pre-fix all four returned
    /// `Ok(SENTINEL)`: the write never landed, the destination kept whatever
    /// stale bytes it already held, and the program reported success — the
    /// failure mode with no signature to look for (T0112).
    ///
    /// `StoreAligned`/`StoreUnaligned` are the ordinary case, not just the
    /// exotic masked/scatter forms, which is why all four are pinned here.
    #[test]
    fn every_simd_store_sub_op_refuses_instead_of_dropping_the_write() {
        for sub_op in [
            SimdSubOpcode::StoreAligned,
            SimdSubOpcode::StoreUnaligned,
            SimdSubOpcode::MaskedStore,
            SimdSubOpcode::Scatter,
        ] {
            let result = run_carrier_then_sentinel(store_carrier(sub_op));
            match result {
                Err(InterpreterError::NotImplemented { feature, opcode }) => {
                    assert!(
                        feature.starts_with("simd_") && feature.contains("store")
                            || feature == "simd_scatter",
                        "{sub_op:?} must name itself in the diagnostic; got {feature:?}",
                    );
                    assert_eq!(opcode, Some(Opcode::SimdExtended));
                }
                Err(other) => panic!("{sub_op:?}: expected NotImplemented, got {other:?}"),
                Ok(value) => panic!(
                    "{sub_op:?} completed and returned {}: the store was compiled, run and \
                     reported successful while writing NOTHING. A dropped write leaves the \
                     destination holding stale data (T0112).",
                    value.as_i64()
                ),
            }
        }
    }

    /// The refusal must be narrow: the value ops of the same family still
    /// compute, and this observes the RESULT, not the shape. 2.0 + 3.0 = 5.0
    /// through `SimdExtended{Add}` proves the envelope, the operand reads and
    /// the neighbouring arms are untouched.
    #[test]
    fn simd_add_still_produces_its_value() {
        let result = run(&[
            Instruction::LoadF {
                dst: Reg(1),
                value: 2.0,
            },
            Instruction::LoadF {
                dst: Reg(2),
                value: 3.0,
            },
            Instruction::SimdExtended {
                sub_op: SimdSubOpcode::Add as u8,
                operands: vec![0, 1, 2],
            },
            Instruction::Ret { value: Reg(0) },
        ])
        .expect("SimdExtended{Add} must still execute");
        assert_eq!(
            result.as_f64(),
            5.0,
            "scalar-fallback SIMD add must still compute its value",
        );
    }

    /// Every SIMD LOAD sub-op refuses too (T0184). Pre-fix all four returned
    /// `Ok(SENTINEL)` having set `dst = ptr` — the ADDRESS ITSELF as the
    /// loaded value, never a dereference. That is the fabricated-data half of
    /// the same class: a plausible number that flows onward indistinguishable
    /// from a real lane.
    ///
    /// This became safe to land only after `core/simd/bytes.vr::find_byte` was
    /// rerouted off the SIMD path (70e988843): before that, `find_byte` was
    /// the single live caller and a loud load would have aborted a shipped
    /// stdlib function on every call.
    #[test]
    fn every_simd_load_sub_op_refuses_instead_of_answering_with_the_pointer() {
        for sub_op in [
            SimdSubOpcode::LoadAligned,
            SimdSubOpcode::LoadUnaligned,
            SimdSubOpcode::MaskedLoad,
            SimdSubOpcode::Gather,
        ] {
            let result = run_carrier_then_sentinel(Instruction::SimdExtended {
                sub_op: sub_op as u8,
                operands: vec![1, 2, 3],
            });
            match result {
                Err(InterpreterError::NotImplemented { feature, opcode }) => {
                    assert!(
                        feature.starts_with("simd_"),
                        "{sub_op:?} must name itself in the diagnostic; got {feature:?}",
                    );
                    assert_eq!(opcode, Some(Opcode::SimdExtended));
                }
                Err(other) => panic!("{sub_op:?}: expected NotImplemented, got {other:?}"),
                Ok(value) => panic!(
                    "{sub_op:?} completed and returned {}: the load answered with the POINTER \
                     as data rather than reading memory (T0184).",
                    value.as_i64()
                ),
            }
        }
    }

    /// The refusal covers memory ops ONLY. A width-1 shuffle really is the
    /// identity, so it keeps its scalar fallback and a program containing one
    /// still runs to completion. Pinning this stops a future widening of the
    /// refusal from swallowing the arms that are honest at width 1.
    #[test]
    fn simd_shuffle_still_completes() {
        let value = run_carrier_then_sentinel(Instruction::SimdExtended {
            sub_op: SimdSubOpcode::Shuffle as u8,
            operands: vec![1, 2, 3],
        })
        .expect("SimdExtended{Shuffle} must still execute");
        assert_eq!(value.as_i64(), SENTINEL as i64);
    }
}
