//! Arithmetic extended opcode handler for VBC interpreter dispatch.

use crate::instruction::ArithSubOpcode;
use crate::types::TypeId;
use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;
use super::arith_helpers::*;

/// ArithExtended (0xBD) - Extended arithmetic operations.
///
/// Sub-opcodes:
/// - 0x00-0x03: Checked arithmetic (returns Maybe<Int>)
/// - 0x10-0x12: Overflowing arithmetic (returns (result, overflowed))
/// - 0x20-0x25: Polymorphic arithmetic (type-dispatched)
pub(in super::super) fn handle_arith_extended(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let sub_op_byte = read_u8(state)?;
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

            match a.checked_add(b) {
                Some(result) => {
                    // Some(result) - tag = 0, field_count = 1
                    let obj = state.heap.alloc_with_init(
                        TypeId(0x8000), // Maybe.Some variant (tag=0)
                        8 + std::mem::size_of::<Value>(),
                        |data| {
                            let tag_ptr = data.as_mut_ptr() as *mut u32;
                            unsafe {
                                *tag_ptr = 0;           // tag
                                *tag_ptr.add(1) = 1;    // field_count
                            }
                        },
                    )?;
                    state.record_allocation();
                    let field_offset = super::super::super::heap::OBJECT_HEADER_SIZE + 8;
                    let field_ptr = unsafe { (obj.as_ptr() as *mut u8).add(field_offset) as *mut Value };
                    unsafe { *field_ptr = Value::from_i64(result); }
                    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
                }
                None => {
                    // None - tag = 1, field_count = 0
                    // data_size must match MakeVariant(tag=1, field_count=0) for deep_value_eq
                    let obj = state.heap.alloc_with_init(
                        TypeId(0x8001), // Maybe.None variant (tag=1)
                        8, // tag + field_count only, no payload
                        |data| {
                            let tag_ptr = data.as_mut_ptr() as *mut u32;
                            unsafe {
                                *tag_ptr = 1;           // tag
                                *tag_ptr.add(1) = 0;    // field_count
                            }
                        },
                    )?;
                    state.record_allocation();
                    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
                }
            }
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::CheckedSubI) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;

            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64();

            match a.checked_sub(b) {
                Some(result) => {
                    let obj = state.heap.alloc_with_init(
                        TypeId(0x8000),
                        8 + std::mem::size_of::<Value>(),
                        |data| {
                            let tag_ptr = data.as_mut_ptr() as *mut u32;
                            unsafe {
                                *tag_ptr = 0;
                                *tag_ptr.add(1) = 1;
                            }
                        },
                    )?;
                    state.record_allocation();
                    let field_offset = super::super::super::heap::OBJECT_HEADER_SIZE + 8;
                    let field_ptr = unsafe { (obj.as_ptr() as *mut u8).add(field_offset) as *mut Value };
                    unsafe { *field_ptr = Value::from_i64(result); }
                    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
                }
                None => {
                    let obj = state.heap.alloc_with_init(
                        TypeId(0x8001),
                        8,
                        |data| {
                            let tag_ptr = data.as_mut_ptr() as *mut u32;
                            unsafe {
                                *tag_ptr = 1;
                                *tag_ptr.add(1) = 0;
                            }
                        },
                    )?;
                    state.record_allocation();
                    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
                }
            }
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::CheckedMulI) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;

            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64();

            match a.checked_mul(b) {
                Some(result) => {
                    let obj = state.heap.alloc_with_init(
                        TypeId(0x8000),
                        8 + std::mem::size_of::<Value>(),
                        |data| {
                            let tag_ptr = data.as_mut_ptr() as *mut u32;
                            unsafe {
                                *tag_ptr = 0;
                                *tag_ptr.add(1) = 1;
                            }
                        },
                    )?;
                    state.record_allocation();
                    let field_offset = super::super::super::heap::OBJECT_HEADER_SIZE + 8;
                    let field_ptr = unsafe { (obj.as_ptr() as *mut u8).add(field_offset) as *mut Value };
                    unsafe { *field_ptr = Value::from_i64(result); }
                    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
                }
                None => {
                    let obj = state.heap.alloc_with_init(
                        TypeId(0x8001),
                        8,
                        |data| {
                            let tag_ptr = data.as_mut_ptr() as *mut u32;
                            unsafe {
                                *tag_ptr = 1;
                                *tag_ptr.add(1) = 0;
                            }
                        },
                    )?;
                    state.record_allocation();
                    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
                }
            }
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::CheckedDivI) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;

            let a = state.get_reg(a_reg).as_i64();
            let b = state.get_reg(b_reg).as_i64();

            match a.checked_div(b) {
                Some(result) => {
                    let obj = state.heap.alloc_with_init(
                        TypeId(0x8000),
                        8 + std::mem::size_of::<Value>(),
                        |data| {
                            let tag_ptr = data.as_mut_ptr() as *mut u32;
                            unsafe {
                                *tag_ptr = 0;
                                *tag_ptr.add(1) = 1;
                            }
                        },
                    )?;
                    state.record_allocation();
                    let field_offset = super::super::super::heap::OBJECT_HEADER_SIZE + 8;
                    let field_ptr = unsafe { (obj.as_ptr() as *mut u8).add(field_offset) as *mut Value };
                    unsafe { *field_ptr = Value::from_i64(result); }
                    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
                }
                None => {
                    let obj = state.heap.alloc_with_init(
                        TypeId(0x8001),
                        8,
                        |data| {
                            let tag_ptr = data.as_mut_ptr() as *mut u32;
                            unsafe {
                                *tag_ptr = 1;
                                *tag_ptr.add(1) = 0;
                            }
                        },
                    )?;
                    state.record_allocation();
                    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
                }
            }
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

            match a.checked_add(b) {
                Some(result) => {
                    let obj = state.heap.alloc_with_init(
                        TypeId(0x8000), // Maybe.Some variant (tag=0)
                        8 + std::mem::size_of::<Value>(),
                        |data| {
                            let tag_ptr = data.as_mut_ptr() as *mut u32;
                            unsafe {
                                *tag_ptr = 0;           // tag
                                *tag_ptr.add(1) = 1;    // field_count
                            }
                        },
                    )?;
                    state.record_allocation();
                    let field_offset = super::super::super::heap::OBJECT_HEADER_SIZE + 8;
                    let field_ptr = unsafe { (obj.as_ptr() as *mut u8).add(field_offset) as *mut Value };
                    unsafe { *field_ptr = Value::from_i64(result as i64); }
                    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
                }
                None => {
                    let obj = state.heap.alloc_with_init(
                        TypeId(0x8001), // Maybe.None variant (tag=1)
                        8,
                        |data| {
                            let tag_ptr = data.as_mut_ptr() as *mut u32;
                            unsafe {
                                *tag_ptr = 1;           // tag
                                *tag_ptr.add(1) = 0;    // field_count
                            }
                        },
                    )?;
                    state.record_allocation();
                    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
                }
            }
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::CheckedSubU) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;

            let a = state.get_reg(a_reg).as_i64() as u64;
            let b = state.get_reg(b_reg).as_i64() as u64;

            match a.checked_sub(b) {
                Some(result) => {
                    let obj = state.heap.alloc_with_init(
                        TypeId(0x8000),
                        8 + std::mem::size_of::<Value>(),
                        |data| {
                            let tag_ptr = data.as_mut_ptr() as *mut u32;
                            unsafe {
                                *tag_ptr = 0;
                                *tag_ptr.add(1) = 1;
                            }
                        },
                    )?;
                    state.record_allocation();
                    let field_offset = super::super::super::heap::OBJECT_HEADER_SIZE + 8;
                    let field_ptr = unsafe { (obj.as_ptr() as *mut u8).add(field_offset) as *mut Value };
                    unsafe { *field_ptr = Value::from_i64(result as i64); }
                    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
                }
                None => {
                    let obj = state.heap.alloc_with_init(
                        TypeId(0x8001),
                        8,
                        |data| {
                            let tag_ptr = data.as_mut_ptr() as *mut u32;
                            unsafe {
                                *tag_ptr = 1;
                                *tag_ptr.add(1) = 0;
                            }
                        },
                    )?;
                    state.record_allocation();
                    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
                }
            }
            Ok(DispatchResult::Continue)
        }

        Some(ArithSubOpcode::CheckedMulU) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;

            let a = state.get_reg(a_reg).as_i64() as u64;
            let b = state.get_reg(b_reg).as_i64() as u64;

            match a.checked_mul(b) {
                Some(result) => {
                    let obj = state.heap.alloc_with_init(
                        TypeId(0x8000),
                        8 + std::mem::size_of::<Value>(),
                        |data| {
                            let tag_ptr = data.as_mut_ptr() as *mut u32;
                            unsafe {
                                *tag_ptr = 0;
                                *tag_ptr.add(1) = 1;
                            }
                        },
                    )?;
                    state.record_allocation();
                    let field_offset = super::super::super::heap::OBJECT_HEADER_SIZE + 8;
                    let field_ptr = unsafe { (obj.as_ptr() as *mut u8).add(field_offset) as *mut Value };
                    unsafe { *field_ptr = Value::from_i64(result as i64); }
                    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
                }
                None => {
                    let obj = state.heap.alloc_with_init(
                        TypeId(0x8001),
                        8,
                        |data| {
                            let tag_ptr = data.as_mut_ptr() as *mut u32;
                            unsafe {
                                *tag_ptr = 1;
                                *tag_ptr.add(1) = 0;
                            }
                        },
                    )?;
                    state.record_allocation();
                    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
                }
            }
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
            let obj = state.heap.alloc(TypeId::UNIT, 2 * std::mem::size_of::<Value>())?;
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

            let obj = state.heap.alloc(TypeId::UNIT, 2 * std::mem::size_of::<Value>())?;
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

            let obj = state.heap.alloc(TypeId::UNIT, 2 * std::mem::size_of::<Value>())?;
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

            eprintln!("[DEBUG PolyAbs] dst={:?} src_reg={:?} is_float={} is_int={}",
                dst, src_reg, src.is_float(), src.is_int());

            let result = if src.is_float() {
                eprintln!("[DEBUG PolyAbs] Float path: {} -> {}", src.as_f64(), src.as_f64().abs());
                Value::from_f64(src.as_f64().abs())
            } else {
                // Use wrapping_abs to handle MIN value correctly
                eprintln!("[DEBUG PolyAbs] Int path: {} -> {}", src.as_i64(), src.as_i64().wrapping_abs());
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

        None => {
            Err(InterpreterError::NotImplemented {
                feature: "unknown arithmetic sub-opcode",
                opcode: None,
            })
        }
    }
}
