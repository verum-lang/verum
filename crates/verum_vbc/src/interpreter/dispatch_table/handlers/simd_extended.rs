//! SIMD extended opcode handler for VBC interpreter dispatch.

use crate::instruction::{SimdSubOpcode, Opcode};
use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;

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
/// Note: SIMD operations require platform-specific support. The interpreter
/// implements scalar fallbacks; AOT compilation uses LLVM vector intrinsics.
pub(in super::super) fn handle_simd_extended(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let sub_op_byte = read_u8(state)?;
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
            let _lane = read_u8(state)?;
            // Scalar fallback: just return the value
            let val = state.get_reg(src_reg);
            state.set_reg(dst, val);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Insert) => {
            // Insert into single lane
            let dst = read_reg(state)?;
            let vec_reg = read_reg(state)?;
            let _lane = read_u8(state)?;
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
        // ================================================================
        Some(SimdSubOpcode::LoadAligned) | Some(SimdSubOpcode::LoadUnaligned) => {
            // Vector load from memory
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            // Scalar fallback: load single value
            let ptr_val = state.get_reg(ptr_reg);
            state.set_reg(dst, ptr_val);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::StoreAligned) | Some(SimdSubOpcode::StoreUnaligned) => {
            // Vector store to memory
            let src_reg = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            // Scalar fallback: no-op in interpreter
            let _ = state.get_reg(src_reg);
            let _ = state.get_reg(ptr_reg);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::MaskedLoad) => {
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let _mask_reg = read_reg(state)?;
            let ptr_val = state.get_reg(ptr_reg);
            state.set_reg(dst, ptr_val);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::MaskedStore) => {
            let src_reg = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let _mask_reg = read_reg(state)?;
            let _ = state.get_reg(src_reg);
            let _ = state.get_reg(ptr_reg);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Gather) => {
            // Gather from indices
            let dst = read_reg(state)?;
            let base_reg = read_reg(state)?;
            let _indices_reg = read_reg(state)?;
            // Scalar fallback: just use base
            let base = state.get_reg(base_reg);
            state.set_reg(dst, base);
            Ok(DispatchResult::Continue)
        }

        Some(SimdSubOpcode::Scatter) => {
            // Scatter to indices
            let src_reg = read_reg(state)?;
            let _base_reg = read_reg(state)?;
            let _indices_reg = read_reg(state)?;
            let _ = state.get_reg(src_reg);
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Shuffle/Permute (0x60-0x6F)
        // ================================================================
        Some(SimdSubOpcode::Shuffle) => {
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
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let _amount = read_u8(state)?;
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
        None => {
            Err(InterpreterError::NotImplemented {
                feature: "simd_extended sub-opcode",
                opcode: Some(Opcode::SimdExtended),
            })
        }
    }
}
