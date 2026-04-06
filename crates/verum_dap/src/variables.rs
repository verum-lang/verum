//! Variable inspection for the DAP server.
//!
//! Reads VBC registers and formats values for display in the IDE debugger.

use verum_vbc::interpreter::{CallFrame, InterpreterState};
use verum_vbc::module::VbcModule;
use verum_vbc::value::Value;

use crate::types::Variable;

/// A variables reference handle.
///
/// The DAP protocol uses `variablesReference` integers to identify variable containers.
/// We encode frame index and scope type into a single i64:
///   - Bits 0..31: frame index
///   - Bits 32..33: scope kind (0 = locals, 1 = arguments)
pub fn encode_variables_reference(frame_index: i64, scope_kind: i64) -> i64 {
    (scope_kind << 32) | (frame_index & 0xFFFF_FFFF)
}

/// Decodes a variables reference back to (frame_index, scope_kind).
pub fn decode_variables_reference(reference: i64) -> (i64, i64) {
    let frame_index = reference & 0xFFFF_FFFF;
    let scope_kind = (reference >> 32) & 0x3;
    (frame_index, scope_kind)
}

/// Reads variables for a given stack frame from the interpreter state.
///
/// Uses `FunctionDescriptor.debug_variables` to map register indices to names.
pub fn read_frame_variables(
    state: &InterpreterState,
    module: &VbcModule,
    frame: &CallFrame,
    scope_kind: i64,
) -> Vec<Variable> {
    let func_desc = module.functions.iter().find(|f| f.id == frame.function);
    let func_desc = match func_desc {
        Some(f) => f,
        None => return Vec::new(),
    };

    let debug_vars = &func_desc.debug_variables;
    if debug_vars.is_empty() {
        return read_registers_fallback(state, frame, func_desc.register_count);
    }

    let mut variables = Vec::new();

    for var_info in debug_vars {
        // Filter by scope kind: 0 = locals (non-parameter), 1 = arguments (parameter).
        let is_param = var_info.is_parameter;
        if (scope_kind == 0 && is_param) || (scope_kind == 1 && !is_param) {
            continue;
        }

        let var_name = module
            .get_string(var_info.name)
            .unwrap_or("<unknown>")
            .to_string();

        // Read the register value using absolute indexing.
        let abs_idx = frame.reg_base + var_info.register as u32;
        let value = state.registers.get_absolute(abs_idx);

        variables.push(Variable {
            name: var_name,
            value: format_value(value),
            ty: Some(infer_value_type(value)),
            variables_reference: 0,
        });
    }

    variables
}

/// Fallback: if no debug variable info is available, show raw registers.
fn read_registers_fallback(
    state: &InterpreterState,
    frame: &CallFrame,
    register_count: u16,
) -> Vec<Variable> {
    let mut variables = Vec::new();
    let count = register_count.min(32); // Cap to avoid flooding the UI.

    for i in 0..count {
        let abs_idx = frame.reg_base + i as u32;
        let value = state.registers.get_absolute(abs_idx);

        // Skip uninitialized/zero registers.
        if value.is_unit() {
            continue;
        }

        variables.push(Variable {
            name: format!("r{}", i),
            value: format_value(value),
            ty: Some(infer_value_type(value)),
            variables_reference: 0,
        });
    }

    variables
}

/// Formats a VBC Value for display.
fn format_value(value: Value) -> String {
    if value.is_int() {
        format!("{}", value.as_i64())
    } else if value.is_float() {
        format!("{}", value.as_f64())
    } else if value.is_bool() {
        format!("{}", value.as_bool())
    } else if value.is_nil() {
        "None".to_string()
    } else if value.is_unit() {
        "()".to_string()
    } else if value.is_small_string() {
        let ss = value.as_small_string();
        format!("\"{}\"", ss.as_str())
    } else if value.is_ptr() {
        format!("ptr(0x{:x})", value.as_ptr::<u8>() as usize)
    } else {
        format!("{:?}", value)
    }
}

/// Infers a display type name from a VBC Value.
fn infer_value_type(value: Value) -> String {
    if value.is_int() {
        "Int".to_string()
    } else if value.is_float() {
        "Float".to_string()
    } else if value.is_bool() {
        "Bool".to_string()
    } else if value.is_nil() {
        "None".to_string()
    } else if value.is_unit() {
        "Unit".to_string()
    } else if value.is_small_string() {
        "Text".to_string()
    } else if value.is_ptr() {
        "Pointer".to_string()
    } else {
        "Unknown".to_string()
    }
}
