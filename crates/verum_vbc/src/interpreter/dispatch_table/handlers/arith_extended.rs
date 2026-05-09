//! Arithmetic extended opcode handler for VBC interpreter dispatch.

use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::arith_helpers::*;
use super::bytecode_io::*;
use crate::instruction::ArithSubOpcode;
use crate::types::TypeId;
use crate::value::Value;

/// ArithExtended (0xBD) - Extended arithmetic operations.
///

/// Sub-opcodes:
/// - 0x00-0x03: Checked arithmetic (returns Maybe<Int>)
/// - 0x10-0x12: Overflowing arithmetic (returns (result, overflowed))
/// - 0x20-0x25: Polymorphic arithmetic (type-dispatched)
pub(in super::super) fn handle_arith_extended(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let sub_op_byte = read_u8(state)?;
    // Skip operand-length varint (see encode_instruction's
    // `Instruction::ArithExtended` arm).
    let _operand_len = read_varint(state)?;
    let sub_op = ArithSubOpcode::from_byte(sub_op_byte);

    match sub_op {
        // ================================================================
        // Checked Arithmetic (0x00-0x03) - Returns Maybe<Int>
        // ================================================================
        Some(ArithSubOpcode::CheckedAddI) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64();
            emit_maybe(state, dst, a.checked_add(b).map(Value::from_i64))?;
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::CheckedSubI) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64();
            emit_maybe(state, dst, a.checked_sub(b).map(Value::from_i64))?;
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::CheckedMulI) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64();
            emit_maybe(state, dst, a.checked_mul(b).map(Value::from_i64))?;
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::CheckedDivI) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64();
            emit_maybe(state, dst, a.checked_div(b).map(Value::from_i64))?;
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Checked Arithmetic - Unsigned (0x04-0x06) - Returns Maybe<UInt64>
        // ================================================================
        Some(ArithSubOpcode::CheckedAddU) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_i64() as u64;
            let b = state.get_reg(b_reg).as_i64() as u64;
            emit_maybe(
                state,
                dst,
                a.checked_add(b).map(|r| Value::from_i64(r as i64)),
            )?;
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::CheckedSubU) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_i64() as u64;
            let b = state.get_reg(b_reg).as_i64() as u64;
            emit_maybe(
                state,
                dst,
                a.checked_sub(b).map(|r| Value::from_i64(r as i64)),
            )?;
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::CheckedMulU) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_i64() as u64;
            let b = state.get_reg(b_reg).as_i64() as u64;
            emit_maybe(
                state,
                dst,
                a.checked_mul(b).map(|r| Value::from_i64(r as i64)),
            )?;
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Checked unary signed (#100, task #25)
        //

        // Both ops produce `Maybe<T>`: `Some(value)` for the typical
        // case, `None` for the unique-overflow case (`T::MIN` for
        // signed). Format mirrors WrappingNeg: `dst, src, width, signed`.
        // ================================================================
        Some(ArithSubOpcode::CheckedNeg) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let width = read_u8(state)?;
            let signed = read_u8(state)? != 0;

            let src = state.get_reg(src_reg).as_i64();
            let (result, ok) = checked_neg(src, width, signed);
            emit_maybe_int(state, dst, result, ok)?;
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::CheckedAbs) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let width = read_u8(state)?;
            let signed = read_u8(state)? != 0;

            let src = state.get_reg(src_reg).as_i64();
            let (result, ok) = checked_abs(src, width, signed);
            emit_maybe_int(state, dst, result, ok)?;
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Overflowing Arithmetic (0x10-0x12) - Returns (result, overflowed)
        // ================================================================
        Some(ArithSubOpcode::OverflowingAddI) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;

            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64();
            let (result, overflowed) = a.overflowing_add(b);

            // Allocate tuple (Int, Bool)
            let obj = state
                .heap
                .alloc(TypeId::UNIT, 2 * std::mem::size_of::<Value>())?;
            state.record_allocation();
            let base_ptr = obj.as_ptr() as *mut u8;
            let field0_offset = super::super::super::heap::OBJECT_HEADER_SIZE;
            let field1_offset = field0_offset + std::mem::size_of::<Value>();
            unsafe {
                *(base_ptr.add(field0_offset) as *mut Value) = Value::from_i64(result);
                *(base_ptr.add(field1_offset) as *mut Value) = Value::from_bool(overflowed);
            }
            state.set_reg(dst, Value::from_ptr(base_ptr));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::OverflowingSubI) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;

            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64();
            let (result, overflowed) = a.overflowing_sub(b);

            let obj = state
                .heap
                .alloc(TypeId::UNIT, 2 * std::mem::size_of::<Value>())?;
            state.record_allocation();
            let base_ptr = obj.as_ptr() as *mut u8;
            let field0_offset = super::super::super::heap::OBJECT_HEADER_SIZE;
            let field1_offset = field0_offset + std::mem::size_of::<Value>();
            unsafe {
                *(base_ptr.add(field0_offset) as *mut Value) = Value::from_i64(result);
                *(base_ptr.add(field1_offset) as *mut Value) = Value::from_bool(overflowed);
            }
            state.set_reg(dst, Value::from_ptr(base_ptr));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::OverflowingMulI) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;

            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64();
            let (result, overflowed) = a.overflowing_mul(b);

            let obj = state
                .heap
                .alloc(TypeId::UNIT, 2 * std::mem::size_of::<Value>())?;
            state.record_allocation();
            let base_ptr = obj.as_ptr() as *mut u8;
            let field0_offset = super::super::super::heap::OBJECT_HEADER_SIZE;
            let field1_offset = field0_offset + std::mem::size_of::<Value>();
            unsafe {
                *(base_ptr.add(field0_offset) as *mut Value) = Value::from_i64(result);
                *(base_ptr.add(field1_offset) as *mut Value) = Value::from_bool(overflowed);
            }
            state.set_reg(dst, Value::from_ptr(base_ptr));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Polymorphic Arithmetic (0x20-0x25) - Type-dispatched
        // ================================================================
        Some(ArithSubOpcode::PolyAdd) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;

            let a = state.get_reg(a_reg);
            let b = state.get_reg(b_reg);

            let result = if a.is_float() {
                Value::from_f64(a.as_f64() + b.as_f64())
            } else {
                Value::from_i64(a.as_i64().wrapping_add(b.as_i64()))
            };
            state.set_reg(dst, result);
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::PolySub) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;

            let a = state.get_reg(a_reg);
            let b = state.get_reg(b_reg);

            let result = if a.is_float() {
                Value::from_f64(a.as_f64() - b.as_f64())
            } else {
                Value::from_i64(a.as_i64().wrapping_sub(b.as_i64()))
            };
            state.set_reg(dst, result);
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::PolyMul) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;

            let a = state.get_reg(a_reg);
            let b = state.get_reg(b_reg);

            let result = if a.is_float() {
                Value::from_f64(a.as_f64() * b.as_f64())
            } else {
                Value::from_i64(a.as_i64().wrapping_mul(b.as_i64()))
            };
            state.set_reg(dst, result);
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::PolyDiv) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;

            let a = state.get_reg(a_reg);
            let b = state.get_reg(b_reg);

            let result = if a.is_float() {
                Value::from_f64(a.as_f64() / b.as_f64())
            } else {
                let divisor = b.as_i64();
                if divisor == 0 {
                    return Err(InterpreterError::DivisionByZero);
                }
                Value::from_i64(a.as_i64().wrapping_div(divisor))
            };
            state.set_reg(dst, result);
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::PolyNeg) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src = state.get_reg(src_reg);

            let result = if src.is_float() {
                Value::from_f64(-src.as_f64())
            } else {
                Value::from_i64(src.as_i64().wrapping_neg())
            };
            state.set_reg(dst, result);
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::PolyRem) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;

            let a = state.get_reg(a_reg);
            let b = state.get_reg(b_reg);

            let result = if a.is_float() {
                Value::from_f64(a.as_f64() % b.as_f64())
            } else {
                let divisor = b.as_i64();
                if divisor == 0 {
                    return Err(InterpreterError::DivisionByZero);
                }
                Value::from_i64(a.as_i64().wrapping_rem(divisor))
            };
            state.set_reg(dst, result);
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::PolyAbs) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src = state.get_reg(src_reg);

            // (trace gated on VBC_POLY_TRACE — unconditional eprintln! here
            // previously polluted every program's stderr on every abs().)
            let trace = std::env::var_os("VBC_POLY_TRACE").is_some();
            if trace {
                eprintln!(
                    "[DEBUG PolyAbs] dst={:?} src_reg={:?} is_float={} is_int={}",
                    dst,
                    src_reg,
                    src.is_float(),
                    src.is_int()
                );
            }

            let result = if src.is_float() {
                if trace {
                    eprintln!(
                        "[DEBUG PolyAbs] Float path: {} -> {}",
                        src.as_f64(),
                        src.as_f64().abs()
                    );
                }
                Value::from_f64(src.as_f64().abs())
            } else {
                // Use wrapping_abs to handle MIN value correctly
                if trace {
                    eprintln!(
                        "[DEBUG PolyAbs] Int path: {} -> {}",
                        src.as_i64(),
                        src.as_i64().wrapping_abs()
                    );
                }
                Value::from_i64(src.as_i64().wrapping_abs())
            };
            state.set_reg(dst, result);
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::PolySignum) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src = state.get_reg(src_reg);

            let result = if src.is_float() {
                let f = src.as_f64();
                let signum = if f > 0.0 {
                    1.0
                } else if f < 0.0 {
                    -1.0
                } else {
                    0.0
                };
                Value::from_f64(signum)
            } else {
                let i = src.as_i64();
                let signum = if i > 0 {
                    1
                } else if i < 0 {
                    -1
                } else {
                    0
                };
                Value::from_i64(signum)
            };
            state.set_reg(dst, result);
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::PolyMin) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;

            let a = state.get_reg(a_reg);
            let b = state.get_reg(b_reg);

            let result = if a.is_float() {
                Value::from_f64(a.as_f64().min(b.as_f64()))
            } else {
                Value::from_i64(a.as_i64().min(b.as_i64()))
            };
            state.set_reg(dst, result);
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::PolyMax) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;

            let a = state.get_reg(a_reg);
            let b = state.get_reg(b_reg);

            let result = if a.is_float() {
                Value::from_f64(a.as_f64().max(b.as_f64()))
            } else {
                Value::from_i64(a.as_i64().max(b.as_i64()))
            };
            state.set_reg(dst, result);
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::PolyClamp) => {
            let dst = read_reg(state)?;
            let val_reg = read_reg(state)?;
            let min_reg = read_reg(state)?;
            let max_reg = read_reg(state)?;

            let val = state.get_reg(val_reg);
            let min_val = state.get_reg(min_reg);
            let max_val = state.get_reg(max_reg);

            let result = if val.is_float() {
                let v = val.as_f64();
                let lo = min_val.as_f64();
                let hi = max_val.as_f64();
                Value::from_f64(v.max(lo).min(hi))
            } else {
                let v = val.as_i64();
                let lo = min_val.as_i64();
                let hi = max_val.as_i64();
                Value::from_i64(v.max(lo).min(hi))
            };
            state.set_reg(dst, result);
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Saturating Arithmetic (0x30-0x3F)
        // ================================================================
        Some(ArithSubOpcode::SaturatingAdd) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let width = read_u8(state)?;
            let signed = read_u8(state)? != 0;

            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64();

            let result = saturating_add(a, b, width, signed);
            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::SaturatingSub) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let width = read_u8(state)?;
            let signed = read_u8(state)? != 0;

            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64();

            let result = saturating_sub(a, b, width, signed);
            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::SaturatingMul) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let width = read_u8(state)?;
            let signed = read_u8(state)? != 0;

            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64();

            let result = saturating_mul(a, b, width, signed);
            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Saturating signed unary (#100, task #25)
        // ================================================================
        Some(ArithSubOpcode::SaturatingNeg) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let width = read_u8(state)?;
            let signed = read_u8(state)? != 0;

            let src = state.get_reg(src_reg).as_i64();
            let result = saturating_neg(src, width, signed);
            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::SaturatingAbs) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let width = read_u8(state)?;
            let signed = read_u8(state)? != 0;

            let src = state.get_reg(src_reg).as_i64();
            let result = saturating_abs(src, width, signed);
            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Wrapping Arithmetic (0x40-0x4F)
        // ================================================================
        Some(ArithSubOpcode::WrappingAdd) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let width = read_u8(state)?;
            let signed = read_u8(state)? != 0;

            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64();

            let result = wrapping_add(a, b, width, signed);
            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::WrappingSub) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let width = read_u8(state)?;
            let signed = read_u8(state)? != 0;

            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64();

            let result = wrapping_sub(a, b, width, signed);
            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::WrappingMul) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let width = read_u8(state)?;
            let signed = read_u8(state)? != 0;

            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64();

            let result = wrapping_mul(a, b, width, signed);
            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::WrappingNeg) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let width = read_u8(state)?;
            let signed = read_u8(state)? != 0;

            let src = state.get_reg(src_reg).as_i64();

            let result = wrapping_neg(src, width, signed);
            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::WrappingShl) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let width = read_u8(state)?;
            let signed = read_u8(state)? != 0;

            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64() as u32;

            let result = wrapping_shl(a, b, width, signed);
            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::WrappingShr) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let width = read_u8(state)?;
            let signed = read_u8(state)? != 0;

            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64() as u32;

            let result = wrapping_shr(a, b, width, signed);
            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Bit Counting Operations (0x50-0x5F)
        // ================================================================
        Some(ArithSubOpcode::Clz) => {
            // Count leading zeros (64-bit)
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let v = state.get_reg(src).as_i64() as u64;
            let result = v.leading_zeros() as i64;
            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::Ctz) => {
            // Count trailing zeros (64-bit)
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let v = state.get_reg(src).as_i64() as u64;
            let result = v.trailing_zeros() as i64;
            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::Popcnt) => {
            // Population count - count set bits (64-bit)
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let v = state.get_reg(src).as_i64() as u64;
            let result = v.count_ones() as i64;
            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::Bswap) => {
            // Byte swap - reverse byte order (64-bit)
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let v = state.get_reg(src).as_i64() as u64;
            let result = v.swap_bytes() as i64;
            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::BitReverse) => {
            // Bit reverse - reverse all bits (64-bit)
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let v = state.get_reg(src).as_i64() as u64;
            let result = v.reverse_bits() as i64;
            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::RotateLeft) => {
            // Rotate left (64-bit)
            let dst = read_reg(state)?;
            let val_reg = read_reg(state)?;
            let amount_reg = read_reg(state)?;
            let v = state.get_reg(val_reg).as_i64() as u64;
            let amount = state.get_reg(amount_reg).as_i64() as u32;
            let result = v.rotate_left(amount) as i64;
            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::RotateRight) => {
            // Rotate right (64-bit)
            let dst = read_reg(state)?;
            let val_reg = read_reg(state)?;
            let amount_reg = read_reg(state)?;
            let v = state.get_reg(val_reg).as_i64() as u64;
            let amount = state.get_reg(amount_reg).as_i64() as u32;
            let result = v.rotate_right(amount) as i64;
            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Binary Float Operations (0x60-0x67)
        // ================================================================
        Some(ArithSubOpcode::Atan2) => {
            // atan2(y, x) - two-argument arctangent
            let dst = read_reg(state)?;
            let y_reg = read_reg(state)?;
            let x_reg = read_reg(state)?;
            let y = state.get_reg(y_reg).as_f64();
            let x = state.get_reg(x_reg).as_f64();
            let result = y.atan2(x);
            state.set_reg(dst, Value::from_f64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::Hypot) => {
            // hypot(x, y) - sqrt(x² + y²) without overflow
            let dst = read_reg(state)?;
            let x_reg = read_reg(state)?;
            let y_reg = read_reg(state)?;
            let x = state.get_reg(x_reg).as_f64();
            let y = state.get_reg(y_reg).as_f64();
            let result = x.hypot(y);
            state.set_reg(dst, Value::from_f64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::Copysign) => {
            // copysign(mag, sign) - magnitude with sign
            let dst = read_reg(state)?;
            let mag_reg = read_reg(state)?;
            let sign_reg = read_reg(state)?;
            let mag = state.get_reg(mag_reg).as_f64();
            let sign = state.get_reg(sign_reg).as_f64();
            let result = mag.copysign(sign);
            state.set_reg(dst, Value::from_f64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::Pow) => {
            // pow(base, exp) - raise to power
            let dst = read_reg(state)?;
            let base_reg = read_reg(state)?;
            let exp_reg = read_reg(state)?;
            let base = state.get_reg(base_reg).as_f64();
            let exp = state.get_reg(exp_reg).as_f64();
            let result = base.powf(exp);
            state.set_reg(dst, Value::from_f64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::LogBase) => {
            // log(x, base) - logarithm with arbitrary base
            let dst = read_reg(state)?;
            let x_reg = read_reg(state)?;
            let base_reg = read_reg(state)?;
            let x = state.get_reg(x_reg).as_f64();
            let base = state.get_reg(base_reg).as_f64();
            let result = x.log(base);
            state.set_reg(dst, Value::from_f64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::Fmod) => {
            // fmod(x, y) - floating-point remainder (truncated)
            let dst = read_reg(state)?;
            let x_reg = read_reg(state)?;
            let y_reg = read_reg(state)?;
            let x = state.get_reg(x_reg).as_f64();
            let y = state.get_reg(y_reg).as_f64();
            // fmod = x - trunc(x/y) * y
            let result = x - (x / y).trunc() * y;
            state.set_reg(dst, Value::from_f64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::Remainder) => {
            // remainder(x, y) - IEEE 754 remainder (rounded)
            let dst = read_reg(state)?;
            let x_reg = read_reg(state)?;
            let y_reg = read_reg(state)?;
            let x = state.get_reg(x_reg).as_f64();
            let y = state.get_reg(y_reg).as_f64();
            // IEEE 754 remainder = x - round(x/y) * y
            let result = x - (x / y).round() * y;
            state.set_reg(dst, Value::from_f64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::Fdim) => {
            // fdim(x, y) - positive difference: max(x - y, 0)
            let dst = read_reg(state)?;
            let x_reg = read_reg(state)?;
            let y_reg = read_reg(state)?;
            let x = state.get_reg(x_reg).as_f64();
            let y = state.get_reg(y_reg).as_f64();
            let result = if x > y { x - y } else { 0.0 };
            state.set_reg(dst, Value::from_f64(result));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Type Conversions (0x70-0x7F)
        // ================================================================
        Some(ArithSubOpcode::SextI) => {
            // Sign-extend integer from narrower to wider type
            // Format: dst:reg, src:reg, from_bits:u8, to_bits:u8
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let from_bits = read_u8(state)?;
            let _to_bits = read_u8(state)?; // VBC uses 64-bit values, so to_bits is implicit

            let v = state.get_reg(src).as_i64();

            // Sign-extend from from_bits to 64 bits
            let result = match from_bits {
                8 => (v as i8) as i64,
                16 => (v as i16) as i64,
                32 => (v as i32) as i64,
                64 => v, // No extension needed
                _ => v,  // Fallback to identity
            };

            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::ZextI) => {
            // Zero-extend integer from narrower to wider type
            // Format: dst:reg, src:reg, from_bits:u8, to_bits:u8
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let from_bits = read_u8(state)?;
            let _to_bits = read_u8(state)?; // VBC uses 64-bit values, so to_bits is implicit

            let v = state.get_reg(src).as_i64() as u64;

            // Zero-extend by masking to from_bits
            let mask = if from_bits >= 64 {
                u64::MAX
            } else {
                (1u64 << from_bits) - 1
            };
            let result = (v & mask) as i64;

            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::FptruncF) => {
            // Truncate float precision: f64 -> f32
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let src = read_reg(state)?;

            let v = state.get_reg(src).as_f64();
            // Truncate to f32 and back to f64 for storage (VBC uses f64 internally)
            let result = (v as f32) as f64;

            state.set_reg(dst, Value::from_f64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::FpextF) => {
            // Extend float precision: f32 -> f64
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let src = read_reg(state)?;

            // VBC stores f32 as f64, so this is effectively an identity operation
            // but ensures proper representation for f32 values
            let v = state.get_reg(src).as_f64();
            // Ensure the value is in f32 range, then extend
            let result = (v as f32) as f64;

            state.set_reg(dst, Value::from_f64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::IntTrunc) => {
            // Truncate integer to narrower type
            // Format: dst:reg, src:reg, to_bits:u8
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let to_bits = read_u8(state)?;

            let v = state.get_reg(src).as_i64() as u64;

            // Truncate by masking to to_bits
            let mask = if to_bits >= 64 {
                u64::MAX
            } else {
                (1u64 << to_bits) - 1
            };
            let result = (v & mask) as i64;

            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::F32ToBits) => {
            // Reinterpret f32 bits as u32
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let src = read_reg(state)?;

            let v = state.get_reg(src).as_f64();
            let f32_val = v as f32;
            let bits = f32_val.to_bits() as i64;

            state.set_reg(dst, Value::from_i64(bits));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::F32FromBits) => {
            // Reinterpret u32 bits as f32
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let src = read_reg(state)?;

            let bits = state.get_reg(src).as_i64() as u32;
            let f32_val = f32::from_bits(bits);
            let result = f32_val as f64;

            state.set_reg(dst, Value::from_f64(result));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::F64ToBits) => {
            // Reinterpret f64 bits as u64
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let src = read_reg(state)?;

            let v = state.get_reg(src).as_f64();
            let bits = v.to_bits();

            state.set_reg(dst, Value::from_i64(bits as i64));
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::F64FromBits) => {
            // Reinterpret u64 bits as f64
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let src = read_reg(state)?;

            let bits = state.get_reg(src).as_i64() as u64;
            let result = f64::from_bits(bits);

            state.set_reg(dst, Value::from_f64(result));
            Ok(DispatchResult::Continue)
        }

        None => Err(InterpreterError::NotImplemented {
            feature: "unknown arithmetic sub-opcode",
            opcode: None,
        }),
    }
}

/// Emit a `Maybe<Int>` value into `dst` using **canonical**
/// `MAYBE_VARIANT_LAYOUT` tags (`None=0`, `Some=1`).
///
/// Delegates to the shared `method_dispatch::{make_some_value,
/// make_none_value}` builders so values produced here are
/// bit-equivalent to those from the `MakeVariant` opcode and the
/// method-call interception path — pattern-match dispatch and
/// `format_variant_for_print_depth` treat them identically.
///
/// **Drift contract** (pinned by
/// `tests::checked_arith_uses_canonical_maybe_tags`): pre-this-helper
/// the `CheckedAddI` / `CheckedSubI` / … arms inlined the
/// alloc-with-init pattern with `Some → tag=0, None → tag=1` — the
/// OPPOSITE of canonical, which silently mis-tagged every
/// overflow-checked arithmetic result. Pattern-matching `if let
/// Some(x) = checked_add(a, b)` would compile to `MatchVariant {
/// variant_tag: 1 }` and find `tag=0` on the heap, failing the match
/// even on the success branch. Routing through canonical builders
/// makes the bug structurally impossible — a single source of truth
/// for `Maybe<T>` construction across the runtime.
fn emit_maybe_int(
    state: &mut InterpreterState,
    dst: crate::instruction::Reg,
    value: i64,
    ok: bool,
) -> InterpreterResult<()> {
    let payload = ok.then(|| Value::from_i64(value));
    emit_maybe(state, dst, payload)
}

/// Generalised counterpart of [`emit_maybe_int`] that accepts any
/// payload `Value`. Single canonical Maybe constructor for the
/// arith-extended dispatch handlers; same drift contract applies.
fn emit_maybe(
    state: &mut InterpreterState,
    dst: crate::instruction::Reg,
    payload: Option<Value>,
) -> InterpreterResult<()> {
    let val = match payload {
        Some(v) => super::method_dispatch::make_some_value(state, v)?,
        None => super::method_dispatch::make_none_value(state)?,
    };
    state.set_reg(dst, val);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instruction::Reg;
    use crate::interpreter::heap;
    use crate::interpreter::state::InterpreterState;
    use crate::module::VbcModule;
    use std::sync::Arc;
    use verum_common::well_known_types::{
        MAYBE_VARIANT_LAYOUT, maybe_none_tag, maybe_success_tag,
    };

    fn fresh_state() -> InterpreterState {
        let module = Arc::new(VbcModule::new("test_arith_extended".to_string()));
        let mut state = InterpreterState::new(module);
        // Seed register file so we have somewhere to write `dst`.
        state.registers.push_frame(8);
        state
    }

    /// Read back the `(tag, field_count, type_id)` triple from a
    /// variant heap object produced by `emit_maybe` / `emit_maybe_int`.
    /// Mirrors what `format_variant_for_print_depth` and
    /// `dispatch_variant_method` see — pinning these is pinning the
    /// observable shape every downstream consumer reads.
    fn read_variant_header(state: &InterpreterState, reg: Reg) -> (u32, u32, u32) {
        let val = state.registers.get(state.reg_base(), reg);
        let ptr = val.as_ptr::<u8>();
        assert!(!ptr.is_null(), "register {} expected to hold a heap pointer", reg.0);
        let header = unsafe { &*(ptr as *const crate::interpreter::heap::ObjectHeader) };
        let type_id = header.type_id.0;
        // SAFETY: ptr non-null (asserted above) and points to a live
        // variant heap object built via `emit_maybe` / canonical helpers.
        let (tag, field_count) = unsafe { heap::variant_header_pair(ptr) };
        (tag, field_count, type_id)
    }

    /// `emit_maybe` with `Some(_)` produces canonical-tagged
    /// `Maybe.Some` exactly matching `MAYBE_VARIANT_LAYOUT`.  Pre-fix
    /// this returned tag=0; the regression test catches any future
    /// re-inversion at the dispatch handler level.
    #[test]
    fn emit_maybe_some_uses_canonical_tag() {
        let mut state = fresh_state();
        let dst = Reg(0);
        emit_maybe(&mut state, dst, Some(Value::from_i64(42))).unwrap();
        let (tag, fc, type_id) = read_variant_header(&state, dst);
        assert_eq!(
            tag,
            maybe_success_tag(),
            "Some tag must match MAYBE_VARIANT_LAYOUT canonical Some=1; got {tag}",
        );
        assert_eq!(fc, 1, "Some has one payload field");
        assert_eq!(
            type_id,
            0x8000 + maybe_success_tag(),
            "synthetic TypeId must follow `0x8000 + tag` formula",
        );
    }

    /// `emit_maybe` with `None` produces canonical-tagged `Maybe.None`.
    #[test]
    fn emit_maybe_none_uses_canonical_tag() {
        let mut state = fresh_state();
        let dst = Reg(1);
        emit_maybe(&mut state, dst, None).unwrap();
        let (tag, fc, type_id) = read_variant_header(&state, dst);
        assert_eq!(
            tag,
            maybe_none_tag(),
            "None tag must match MAYBE_VARIANT_LAYOUT canonical None=0; got {tag}",
        );
        assert_eq!(fc, 0, "None is unit-shaped (no payload)");
        assert_eq!(
            type_id,
            0x8000 + maybe_none_tag(),
            "synthetic TypeId must follow `0x8000 + tag` formula",
        );
    }

    /// `emit_maybe_int` mirrors `emit_maybe` for the i64-payload case.
    /// Tags must match canonical regardless of which branch fires.
    #[test]
    fn emit_maybe_int_canonical_branches() {
        let mut state = fresh_state();
        emit_maybe_int(&mut state, Reg(2), 7, true).unwrap();
        let (tag_some, fc_some, _) = read_variant_header(&state, Reg(2));
        assert_eq!(tag_some, maybe_success_tag());
        assert_eq!(fc_some, 1);

        emit_maybe_int(&mut state, Reg(3), 0, false).unwrap();
        let (tag_none, fc_none, _) = read_variant_header(&state, Reg(3));
        assert_eq!(tag_none, maybe_none_tag());
        assert_eq!(fc_none, 0);
    }

    /// Cross-handler bit-equivalence: a `Maybe<Int>` produced by the
    /// arith-extended dispatch path is bit-identical to one produced
    /// by `pattern_matching::alloc_variant_into` (the `MakeVariant`
    /// opcode handler).  This is the core invariant that lets
    /// pattern-match code on `Some/None` work uniformly regardless of
    /// which path produced the value.
    #[test]
    fn arith_maybe_matches_makevariant_shape() {
        let mut state = fresh_state();
        // arith path: Some(99)
        emit_maybe(&mut state, Reg(4), Some(Value::from_i64(99))).unwrap();
        let (a_tag, a_fc, a_tid) = read_variant_header(&state, Reg(4));

        // MakeVariant opcode path: same logical Some(99)
        crate::interpreter::dispatch_table::handlers::pattern_matching::alloc_variant_into(
            &mut state,
            Reg(5),
            maybe_success_tag(),
            1,
        )
        .unwrap();
        // Set payload field 0 = 99 (mirrors what SetVariantData does).
        let val5 = state.registers.get(state.reg_base(), Reg(5));
        unsafe {
            let ptr = val5.as_ptr::<u8>();
            let payload_ptr = ptr.add(heap::OBJECT_HEADER_SIZE + 8) as *mut Value;
            *payload_ptr = Value::from_i64(99);
        }
        let (b_tag, b_fc, b_tid) = read_variant_header(&state, Reg(5));

        assert_eq!(a_tag, b_tag, "arith and MakeVariant paths must agree on tag");
        assert_eq!(
            a_fc, b_fc,
            "arith and MakeVariant paths must agree on field_count",
        );
        assert_eq!(
            a_tid, b_tid,
            "arith and MakeVariant paths must agree on synthetic TypeId",
        );
    }

    /// Pin: the canonical layout `MAYBE_VARIANT_LAYOUT` itself is the
    /// source-of-truth this handler trusts.  If a future edit reorders
    /// `core/base/maybe.vr`, the corresponding update to
    /// `MAYBE_VARIANT_LAYOUT` flows through `maybe_success_tag()` /
    /// `maybe_none_tag()` and the assertions above pick up the new
    /// values automatically — no parallel hand-table to drift from.
    #[test]
    fn maybe_layout_canonical_pin() {
        // Two variants total.
        assert_eq!(MAYBE_VARIANT_LAYOUT.len(), 2);
        // None=0, Some=1 in declaration order.
        assert_eq!(maybe_none_tag(), 0);
        assert_eq!(maybe_success_tag(), 1);
        // Tags must be distinct (no collision is structurally impossible
        // here, but the assertion is the contract for downstream consumers).
        assert_ne!(maybe_none_tag(), maybe_success_tag());
    }
}
