//! Meta operation handlers for VBC interpreter dispatch.
//!
//! Handles: MetaEval (0xB8), MetaQuote (0xB9), MetaSplice (0xBA), MetaReflect (0xBB)
//!
//! Verum unified meta-system: all compile-time computation uses `meta fn` and `@` prefix.
//! MetaEval evaluates compile-time expressions, MetaQuote captures code as TokenStream,
//! MetaSplice inserts computed values into generated code, MetaReflect provides type
//! introspection (@type_name, @type_fields, @variants_of, etc.). Everything desugars
//! to meta-system operations (tagged literals, derives, const_eval, interpolation).

use crate::module::ConstId;
use crate::module::Constant;
use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;

// ============================================================================
// Meta Operations
// ============================================================================

/// MetaEval (0xB8) - Evaluate compile-time expression at runtime.
///
/// At runtime, the expression was already evaluated at compile time.
/// Just copy the value through.
pub(in super::super) fn handle_meta_eval(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let expr = read_reg(state)?;
    let value = state.get_reg(expr);
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

/// MetaQuote (0xB9) - Create TokenStream from serialized token data.
pub(in super::super) fn handle_meta_quote(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let bytes_const_id = read_varint(state)? as u32;

    let constant = state
        .module
        .get_constant(ConstId(bytes_const_id))
        .cloned()
        .ok_or_else(|| InterpreterError::InvalidBytecode {
            pc: state.pc() as usize,
            message: format!("Invalid constant id for MetaQuote: {}", bytes_const_id),
        })?;

    let bytes = match constant {
        Constant::Bytes(b) => b,
        _ => {
            return Err(InterpreterError::InvalidBytecode {
                pc: state.pc() as usize,
                message: format!(
                    "MetaQuote expected Constant::Bytes, got {:?}",
                    constant.tag()
                ),
            });
        }
    };

    let obj = state.heap.alloc_token_stream(&bytes)?;
    state.record_allocation();

    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));

    Ok(DispatchResult::Continue)
}

/// MetaSplice (0xBA) - Splice tokenstream into code.
///
/// Compile-time only -- at runtime, spliced code is already in the bytecode.
pub(in super::super) fn handle_meta_splice(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let _src = read_reg(state)?;
    Ok(DispatchResult::Continue)
}

/// MetaReflect (0xBB) - Type introspection operations.
pub(in super::super) fn handle_meta_reflect(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    use crate::instruction::MetaReflectOp;
    use crate::types::TypeId;

    let sub_op_byte = read_u8(state)?;
    let sub_op = MetaReflectOp::from_byte(sub_op_byte);
    let dst = read_reg(state)?;
    let value_reg = read_reg(state)?;
    let value = state.get_reg(value_reg);

    // Helper to get TypeId from Value based on its runtime type
    let get_type_id = |v: Value| -> TypeId {
        if v.is_float() {
            TypeId::FLOAT
        } else if v.is_int() {
            TypeId::INT
        } else if v.is_bool() {
            TypeId::BOOL
        } else if v.is_unit() {
            TypeId::UNIT
        } else if v.is_small_string() {
            TypeId::TEXT
        } else if v.is_type_ref() {
            v.as_type_id()
        } else if v.is_ptr() {
            TypeId::PTR
        } else {
            TypeId::PTR
        }
    };

    // Helper to get type name from TypeId
    let get_type_name = |tid: TypeId| -> &'static str {
        match tid {
            TypeId::UNIT => "Unit",
            TypeId::BOOL => "Bool",
            TypeId::INT => "Int",
            TypeId::FLOAT => "Float",
            TypeId::TEXT => "Text",
            TypeId::NEVER => "Never",
            TypeId::U8 => "U8",
            TypeId::U16 => "U16",
            TypeId::U32 => "U32",
            TypeId::U64 => "U64",
            TypeId::I8 => "I8",
            TypeId::I16 => "I16",
            TypeId::I32 => "I32",
            TypeId::F32 => "F32",
            TypeId::PTR => "Ptr",
            _ => "Unknown",
        }
    };

    let tid = get_type_id(value);

    match sub_op {
        Some(MetaReflectOp::TypeId) => {
            state.set_reg(dst, Value::from_i64(tid.0 as i64));
        }
        Some(MetaReflectOp::TypeName) => {
            let type_name = get_type_name(tid);
            let text_value = Value::from_small_string(type_name)
                .unwrap_or_else(|| {
                    Value::from_small_string("Type").unwrap_or(Value::nil())
                });
            state.set_reg(dst, text_value);
        }
        Some(MetaReflectOp::NeedsDrop) => {
            let needs_drop = value.is_ptr() || value.is_fat_ref() || value.is_thin_ref();
            state.set_reg(dst, Value::from_bool(needs_drop));
        }
        Some(MetaReflectOp::IsCopy) => {
            let is_copy = !value.is_ptr() && !value.is_fat_ref() && !value.is_thin_ref();
            state.set_reg(dst, Value::from_bool(is_copy));
        }
        Some(MetaReflectOp::IsSend) => {
            state.set_reg(dst, Value::from_bool(true));
        }
        Some(MetaReflectOp::IsSync) => {
            state.set_reg(dst, Value::from_bool(true));
        }
        Some(MetaReflectOp::MinAlign) => {
            let align = match tid {
                TypeId::BOOL | TypeId::U8 | TypeId::I8 => 1,
                TypeId::I16 | TypeId::U16 => 2,
                TypeId::INT | TypeId::I32 | TypeId::U32 | TypeId::F32 => 4,
                TypeId::FLOAT | TypeId::U64 | TypeId::PTR => 8,
                _ => 8,
            };
            state.set_reg(dst, Value::from_i64(align as i64));
        }
        Some(MetaReflectOp::PrefAlign) => {
            state.set_reg(dst, Value::from_i64(8));
        }
        None => {
            return Err(InterpreterError::InvalidBytecode {
                pc: state.pc() as usize,
                message: format!("Unknown MetaReflect sub-opcode: 0x{:02X}", sub_op_byte),
            });
        }
    }

    Ok(DispatchResult::Continue)
}
