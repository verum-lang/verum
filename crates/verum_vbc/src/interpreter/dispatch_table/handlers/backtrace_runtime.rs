//! VBC interpreter intercept for `Backtrace.capture()`.
//!
//! `core/base/error.vr`'s `Backtrace.capture()` is a pure-Verum stub that
//! returns an empty backtrace. At Tier 0 (interpreter) we can supply real
//! frames by walking the VBC call stack; no native DWARF/libunwind is needed
//! because the interpreter owns the full call record.
//!
//! ## Frame layout
//!
//! Each Verum `StackFrame` record has 4 fields in declaration order:
//!   [0] function : Text    — qualified function name
//!   [1] file     : Text    — source file (from SourceMap, or "<unknown>")
//!   [2] line     : Int     — source line (1-based, 0 when unavailable)
//!   [3] column   : Int     — source column (1-based, 0 when unavailable)
//!
//! The `Backtrace` record has 1 field:
//!   [0] frames : List<StackFrame>
//!
//! ## Source location resolution
//!
//! If the module carries a `SourceMap`, we binary-search its entries for the
//! largest `bytecode_offset` ≤ `frame.pc` to find the closest source location.
//! Entries are stored in bytecode-offset order (the codegen emits them that way).

use super::super::super::error::InterpreterResult;
use super::super::super::state::InterpreterState;
use super::super::alloc_list_from_values;
use super::heap_helpers::alloc_record_n_fields;
use super::string_helpers::alloc_string_value;
use crate::value::Value;

/// Try to intercept a `Backtrace.capture()` call. Returns `Some(Backtrace)`
/// when the call is recognised, `None` otherwise.
pub(in super::super) fn try_intercept_backtrace(
    state: &mut InterpreterState,
    func_name: &str,
    _args_start_reg: u16,
    arg_count: u8,
    _caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let bare = func_name.rsplit('.').next().unwrap_or(func_name);
    if bare != "capture" {
        return Ok(None);
    }
    // Reject if there are arguments (capture() takes none).
    if arg_count != 0 {
        return Ok(None);
    }
    // Confirm the qualifier ends in "Backtrace.capture".
    if !func_name.ends_with("Backtrace.capture") && func_name != "capture" {
        return Ok(None);
    }

    // Snapshot the call stack before any mutable allocation dirtied it.
    // Each entry is (FunctionId, pc).
    let stack_trace = state.call_stack.stack_trace();

    // Resolve (function_name, file, line, column) for each frame.
    struct FrameInfo {
        func: String,
        file: String,
        line: i64,
        col: i64,
    }

    let mut infos: Vec<FrameInfo> = Vec::with_capacity(stack_trace.len());
    for (func_id, pc) in &stack_trace {
        let func_name_str = state
            .module
            .get_function(*func_id)
            .and_then(|d| state.module.get_string(d.name))
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("<fn:{}>", func_id.0));

        let (file, line, col) = resolve_source_location(&state.module, *pc);
        infos.push(FrameInfo {
            func: func_name_str,
            file,
            line,
            col,
        });
    }

    // Now build the Verum heap objects.
    let mut frame_values: Vec<Value> = Vec::with_capacity(infos.len());
    for info in infos {
        let func_v = alloc_string_value(state, &info.func)?;
        let file_v = alloc_string_value(state, &info.file)?;
        let line_v = Value::from_i64(info.line);
        let col_v = Value::from_i64(info.col);
        let frame = alloc_record_n_fields(state, "StackFrame", &[func_v, file_v, line_v, col_v])?;
        frame_values.push(frame);
    }

    let frames_list = alloc_list_from_values(state, frame_values)?;
    let backtrace = alloc_record_n_fields(state, "Backtrace", &[frames_list])?;
    Ok(Some(backtrace))
}

/// Binary-search the module's SourceMap for the closest entry at or before `pc`.
/// Returns `("<unknown>", 0, 0)` when no source map is present or no entry matches.
fn resolve_source_location(
    module: &crate::module::VbcModule,
    pc: u32,
) -> (String, i64, i64) {
    let sm = match &module.source_map {
        Some(sm) => sm,
        None => return ("<unknown>".into(), 0, 0),
    };
    if sm.entries.is_empty() {
        return ("<unknown>".into(), 0, 0);
    }

    // Binary search for largest bytecode_offset <= pc.
    let idx = sm.entries.partition_point(|e| e.bytecode_offset <= pc);
    let entry = if idx == 0 {
        &sm.entries[0]
    } else {
        &sm.entries[idx - 1]
    };

    let file = sm
        .files
        .get(entry.file_idx as usize)
        .and_then(|fid| module.get_string(*fid))
        .map(|s| s.to_string())
        .unwrap_or_else(|| "<unknown>".into());

    (file, entry.line as i64, entry.column as i64)
}
