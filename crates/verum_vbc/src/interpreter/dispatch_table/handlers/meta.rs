//! Meta operation handlers for VBC interpreter dispatch.
//!
//! Handles: MetaEval (0xB8), MetaQuote (0xB9), MetaSplice (0xBA), MetaReflect (0xBB)
//!
//! Verum unified meta-system: all compile-time computation uses `meta fn` and `@` prefix.
//! MetaEval evaluates compile-time expressions, MetaQuote captures code as TokenStream,
//! MetaSplice inserts computed values into generated code, MetaReflect provides type
//! introspection (@type_name, @type_fields, @variants_of, etc.). Everything desugars
//! to meta-system operations (tagged literals, derives, const_eval, interpolation).

use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;
use crate::module::ConstId;
use crate::module::Constant;
use crate::value::Value;

// ============================================================================
// Meta Operations
// ============================================================================

/// MetaEval (0xB8) - Evaluate compile-time expression at runtime.
///
/// At runtime, the expression was already evaluated at compile time.
/// Just copy the value through.
pub(in super::super) fn handle_meta_eval(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let expr = read_reg(state)?;
    let value = state.get_reg(expr);
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

/// MetaQuote (0xB9) - Create TokenStream from serialized token data.
pub(in super::super) fn handle_meta_quote(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
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
pub(in super::super) fn handle_meta_splice(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let _src = read_reg(state)?;
    Ok(DispatchResult::Continue)
}

/// MetaReflect (0xBB) - Type introspection operations.
pub(in super::super) fn handle_meta_reflect(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    Err(InterpreterError::InvalidBytecode {
        pc: state.pc() as usize,
        message: "legacy MetaReflect lacks a complete type witness; use typed declaration queries".into(),
    })
}
