//! Bytecode reading helpers for VBC interpreter dispatch.

use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use crate::encoding;
use crate::error::VbcResult;
use crate::instruction::{Reg, RegRange};
use crate::types::TypeId;
use crate::value::Value;

// ============================================================================
// Bytecode Reading Helpers (inlined for performance)
// ============================================================================

/// Decode against the active function and publish the new PC only on success.
#[inline(always)]
pub(super) fn read_operand<T>(
    state: &mut InterpreterState,
    decode: impl FnOnce(&[u8], &mut usize) -> VbcResult<T>,
) -> InterpreterResult<T> {
    let pc = state.pc() as usize;
    let bytes = state.operand_bytecode().ok_or_else(|| InterpreterError::InvalidBytecode {
        pc,
        message: "missing current function bytecode".to_string(),
    })?;
    let mut next = pc;
    let value = decode(bytes, &mut next).map_err(|error| InterpreterError::InvalidBytecode {
        pc,
        message: error.to_string(),
    })?;
    // A function descriptor bounds the slice length to u32.
    state.set_pc(next as u32);
    Ok(value)
}

/// Reads a register index from bytecode.
#[inline(always)]
pub(super) fn read_reg(state: &mut InterpreterState) -> InterpreterResult<Reg> {
    read_operand(state, encoding::decode_reg)
}

/// Reads a single byte from bytecode.
#[inline(always)]
pub(super) fn read_u8(state: &mut InterpreterState) -> InterpreterResult<u8> {
    read_operand(state, encoding::decode_u8)
}

/// Reads a signed byte from bytecode.
#[inline(always)]
pub(super) fn read_i8(state: &mut InterpreterState) -> InterpreterResult<i8> {
    Ok(read_u8(state)? as i8)
}

/// Reads a 16-bit unsigned integer from bytecode (little-endian).
#[inline(always)]
pub(super) fn read_u16(state: &mut InterpreterState) -> InterpreterResult<u16> {
    read_operand(state, encoding::decode_u16)
}

/// Reads a 32-bit unsigned integer from bytecode (little-endian).
#[inline(always)]
pub(super) fn read_u32(state: &mut InterpreterState) -> InterpreterResult<u32> {
    read_operand(state, encoding::decode_u32)
}

/// Reads a variable-length unsigned integer (varint) from bytecode.
#[inline(always)]
pub(super) fn read_varint(state: &mut InterpreterState) -> InterpreterResult<u64> {
    read_operand(state, encoding::decode_varint)
}

/// Reads a variable-length signed integer (signed varint) from bytecode.
#[inline(always)]
pub(super) fn read_signed_varint(state: &mut InterpreterState) -> InterpreterResult<i64> {
    read_operand(state, encoding::decode_signed_varint)
}

/// Reads a 64-bit float from bytecode.
#[inline(always)]
pub(super) fn read_f64(state: &mut InterpreterState) -> InterpreterResult<f64> {
    read_operand(state, encoding::decode_f64)
}

/// Reads a register range from bytecode.
#[inline(always)]
pub(super) fn read_reg_range(state: &mut InterpreterState) -> InterpreterResult<RegRange> {
    read_operand(state, encoding::decode_reg_range)
}

/// Decode the full witness using the same structural reader as archive/mono.
/// Runtime dispatch may project a head at its own boundary; witness transport
/// must not erase reference, array, callable, or associated-type structure.
pub(super) fn read_type_ref(state: &mut InterpreterState) -> InterpreterResult<crate::types::TypeRef> {
    read_operand(state, crate::bytecode::decode_type_ref)
}

// ============================================================================
// Tensor Register Helpers
// ============================================================================

/// Extracts a shape (Vec<usize>) from a register value.
/// The register may contain a List of integers or a single integer (1D shape).
pub(super) fn extract_shape_from_register(
    state: &InterpreterState,
    reg: Reg,
) -> InterpreterResult<Vec<usize>> {
    let val = state.get_reg(reg);

    // Single integer → 1D shape
    if val.is_int() {
        return Ok(vec![val.as_i64() as usize]);
    }

    // Pointer → List object
    if val.is_ptr() {
        let ptr = val.as_ptr::<u8>();
        if ptr.is_null() {
            return Ok(vec![]);
        }
        let header = unsafe { &*(ptr as *const crate::interpreter::heap::ObjectHeader) };
        if header.type_id == TypeId::LIST {
            let data_ptr =
                unsafe { ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *const Value };
            let len = unsafe { (*data_ptr).as_i64() } as usize;
            let backing_ptr = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
            if !backing_ptr.is_null() {
                let backing_header =
                    unsafe { &*(backing_ptr as *const crate::interpreter::heap::ObjectHeader) };
                let elem_ptr = unsafe {
                    backing_ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *const Value
                };
                let max_len = len.min(backing_header.size as usize / std::mem::size_of::<Value>());
                let mut shape = Vec::with_capacity(max_len);
                for i in 0..max_len {
                    let elem = unsafe { *elem_ptr.add(i) };
                    shape.push(elem.as_i64() as usize);
                }
                return Ok(shape);
            }
        }
    }

    Ok(vec![])
}

/// Extracts a list of f64 values from a register.
/// If the register contains an integer, returns a single-element list.
pub(super) fn extract_f64_list_from_register(
    state: &InterpreterState,
    reg: Reg,
) -> InterpreterResult<Vec<f64>> {
    let val = state.get_reg(reg);

    if val.is_int() {
        return Ok(vec![val.as_i64() as f64]);
    }
    if val.is_float() {
        return Ok(vec![val.as_f64()]);
    }

    if val.is_ptr() {
        let ptr = val.as_ptr::<u8>();
        if ptr.is_null() {
            return Ok(vec![]);
        }
        let header = unsafe { &*(ptr as *const crate::interpreter::heap::ObjectHeader) };
        if header.type_id == TypeId::LIST {
            let data_ptr =
                unsafe { ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *const Value };
            let len = unsafe { (*data_ptr).as_i64() } as usize;
            let backing_ptr = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
            if !backing_ptr.is_null() {
                let backing_header =
                    unsafe { &*(backing_ptr as *const crate::interpreter::heap::ObjectHeader) };
                let elem_ptr = unsafe {
                    backing_ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *const Value
                };
                let max_len = len.min(backing_header.size as usize / std::mem::size_of::<Value>());
                let mut values = Vec::with_capacity(max_len);
                for i in 0..max_len {
                    let elem = unsafe { *elem_ptr.add(i) };
                    if !elem.is_int() {
                        values.push(elem.as_f64());
                    } else {
                        values.push(elem.as_i64() as f64);
                    }
                }
                return Ok(values);
            }
        }
    }

    Ok(vec![])
}
