//! Bytecode reading helpers for VBC interpreter dispatch.

use crate::instruction::{Reg, RegRange};
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use crate::value::Value;
use crate::types::TypeId;

// ============================================================================
// Bytecode Reading Helpers (inlined for performance)
// ============================================================================

/// Reads a register index from bytecode.
#[inline(always)]
pub(super) fn read_reg(state: &mut InterpreterState) -> InterpreterResult<Reg> {
    let byte = read_u8(state)?;
    if byte < 128 {
        Ok(Reg(byte as u16))
    } else {
        let high = (byte & 0x7F) as u16;
        let low = read_u8(state)? as u16;
        Ok(Reg((high << 8) | low))
    }
}

/// Reads a single byte from bytecode.
#[inline(always)]
pub(super) fn read_u8(state: &mut InterpreterState) -> InterpreterResult<u8> {
    let pc = state.pc();
    let byte = state.read_byte(pc).ok_or_else(|| InterpreterError::InvalidBytecode {
        pc: pc as usize,
        message: "unexpected end of bytecode".to_string(),
    })?;
    state.advance_pc(1);
    Ok(byte)
}

/// Reads a signed byte from bytecode.
#[inline(always)]
pub(super) fn read_i8(state: &mut InterpreterState) -> InterpreterResult<i8> {
    Ok(read_u8(state)? as i8)
}

/// Reads a 16-bit unsigned integer from bytecode (little-endian).
/// Reserved for opcodes that need 16-bit immediate values.
#[allow(dead_code)]
#[inline(always)]
pub(super) fn read_u16(state: &mut InterpreterState) -> InterpreterResult<u16> {
    let b0 = read_u8(state)? as u16;
    let b1 = read_u8(state)? as u16;
    Ok(b0 | (b1 << 8))
}

/// Reads a 32-bit unsigned integer from bytecode (little-endian).
/// Reserved for opcodes that need 32-bit immediate values.
#[allow(dead_code)]
#[inline(always)]
pub(super) fn read_u32(state: &mut InterpreterState) -> InterpreterResult<u32> {
    let b0 = read_u8(state)? as u32;
    let b1 = read_u8(state)? as u32;
    let b2 = read_u8(state)? as u32;
    let b3 = read_u8(state)? as u32;
    Ok(b0 | (b1 << 8) | (b2 << 16) | (b3 << 24))
}

/// Reads a variable-length unsigned integer (varint) from bytecode.
#[inline(always)]
pub(super) fn read_varint(state: &mut InterpreterState) -> InterpreterResult<u64> {
    // Fast path: single-byte varint (values 0-127), most common case
    let byte = read_u8(state)?;
    if byte & 0x80 == 0 {
        return Ok(byte as u64);
    }

    // Multi-byte varint
    let mut result = (byte & 0x7F) as u64;
    let mut shift = 7;
    loop {
        let byte = read_u8(state)?;
        result |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
        if shift >= 64 {
            return Err(InterpreterError::InvalidBytecode {
                pc: state.pc() as usize,
                message: "varint overflow".to_string(),
            });
        }
    }
}

/// Reads a variable-length signed integer (signed varint) from bytecode.
#[inline(always)]
pub(super) fn read_signed_varint(state: &mut InterpreterState) -> InterpreterResult<i64> {
    let unsigned = read_varint(state)?;
    // ZigZag decoding
    Ok(((unsigned >> 1) as i64) ^ -((unsigned & 1) as i64))
}

/// Reads a 64-bit float from bytecode.
#[inline(always)]
pub(super) fn read_f64(state: &mut InterpreterState) -> InterpreterResult<f64> {
    let mut bytes = [0u8; 8];
    for b in &mut bytes {
        *b = read_u8(state)?;
    }
    Ok(f64::from_le_bytes(bytes))
}

/// Reads a register range from bytecode.
#[inline(always)]
pub(super) fn read_reg_range(state: &mut InterpreterState) -> InterpreterResult<RegRange> {
    let start = read_reg(state)?;
    let count = read_u8(state)?;
    Ok(RegRange { start, count })
}

// ============================================================================
// Tensor Register Helpers
// ============================================================================

/// Extracts a shape (Vec<usize>) from a register value.
/// The register may contain a List of integers or a single integer (1D shape).
pub(super) fn extract_shape_from_register(state: &InterpreterState, reg: Reg) -> InterpreterResult<Vec<usize>> {
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
            let data_ptr = unsafe {
                ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *const Value
            };
            let len = unsafe { (*data_ptr).as_i64() } as usize;
            let backing_ptr = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
            if !backing_ptr.is_null() {
                let backing_header = unsafe { &*(backing_ptr as *const crate::interpreter::heap::ObjectHeader) };
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
pub(super) fn extract_f64_list_from_register(state: &InterpreterState, reg: Reg) -> InterpreterResult<Vec<f64>> {
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
            let data_ptr = unsafe {
                ptr.add(crate::interpreter::heap::OBJECT_HEADER_SIZE) as *const Value
            };
            let len = unsafe { (*data_ptr).as_i64() } as usize;
            let backing_ptr = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
            if !backing_ptr.is_null() {
                let backing_header = unsafe { &*(backing_ptr as *const crate::interpreter::heap::ObjectHeader) };
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
