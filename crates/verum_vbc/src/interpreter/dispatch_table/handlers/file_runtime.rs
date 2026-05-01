//! High-level Rust intercepts for `core.io.file` operations.
//!
//! Mirrors the architecture of `shell_runtime.rs` (VBC-1): bypass the
//! libSystem FFI chain for open(2)/read(2)/write(2)/stat(2) syscalls
//! and dispatch directly to `std::fs` from the interpreter host
//! process.  See `shell_runtime.rs` docstring for the full Tier-0
//! architectural rationale.
//!
//! # Functions intercepted
//!
//!   * `read_to_string(path: &Text) -> IoResult<Text>` —
//!     `std::fs::read_to_string`.
//!   * `read(path: &Text) -> IoResult<List<Byte>>` —
//!     `std::fs::read`.
//!   * `write(path: &Text, contents: &Text) -> IoResult<()>` —
//!     `std::fs::write` with text contents.
//!   * `write_bytes(path: &Text, contents: &[Byte]) -> IoResult<()>` —
//!     `std::fs::write` with byte slice.
//!   * `exists(path: &Text) -> Bool` — `std::path::Path::exists`.
//!
//! # Marshaling
//!
//! `IoResult<T>` = `Result<T, StreamError>` where
//! `StreamError { kind: IoErrorKind, message: Maybe<Text> }`.
//! On Rust-side `std::io::Error`, the kind is mapped from
//! `ErrorKind::NotFound` / `PermissionDenied` / etc. to the
//! corresponding `IoErrorKind` variant; an OS-error message goes
//! into `message` (`Maybe.Some(text)`).
//!
//! # Permission gate
//!
//! Consults `PermissionScope::FileSystem` — a script declaring
//! `permissions = ["time"]` (no `fs`) is denied uniformly with the
//! libSystem open/read FFI gate.

use crate::interpreter::heap;
use crate::interpreter::permission::{PermissionDecision, PermissionScope};
use crate::interpreter::state::InterpreterState;
use crate::types::TypeId;
use crate::value::Value;
use super::super::super::error::InterpreterResult;
use super::string_helpers::{alloc_string_value, extract_string};

/// Try to intercept a file-I/O runtime call.  Returns `Some(value)`
/// when the interception fires, `None` otherwise (caller falls through
/// to normal bytecode dispatch).
///
/// Hot-path miss: one string-suffix compare + return None.
pub(in super::super) fn try_intercept_file_runtime(
    state: &mut InterpreterState,
    func_name: &str,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let bare = func_name.rsplit('.').next().unwrap_or(func_name);
    match bare {
        "read_to_string" => intercept_read_to_string(state, args_start_reg, arg_count, caller_base),
        "read" if arg_count == 1 => {
            // Distinguish `core.io.file.read(path)` from other `read(*)`
            // overloads by qualifying-prefix check.
            if !func_name.contains("io.file") && !func_name.contains("io::file") {
                return Ok(None);
            }
            intercept_read_bytes(state, args_start_reg, arg_count, caller_base)
        }
        "write" if arg_count == 2 => {
            if !func_name.contains("io.file") && !func_name.contains("io::file") {
                return Ok(None);
            }
            intercept_write_text(state, args_start_reg, arg_count, caller_base)
        }
        "write_bytes" => intercept_write_bytes(state, args_start_reg, arg_count, caller_base),
        "exists" => {
            if !func_name.contains("io.file") && !func_name.contains("io::file") {
                return Ok(None);
            }
            intercept_exists(state, args_start_reg, arg_count, caller_base)
        }
        _ => Ok(None),
    }
}

// ============================================================================
// Per-function intercepts
// ============================================================================

fn intercept_read_to_string(
    state: &mut InterpreterState,
    args_start_reg: u16,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    if arg_count == 0 {
        return Ok(None);
    }
    let path = extract_path_arg(state, args_start_reg, caller_base);
    if let Some(denied) = check_fs_permission(state, "read") {
        return Ok(Some(denied));
    }
    match std::fs::read_to_string(&path) {
        Ok(s) => {
            let text = alloc_string_value(state, &s)?;
            Ok(Some(wrap_in_variant(state, "Result", 0, &[text])?))
        }
        Err(e) => Ok(Some(build_io_err(state, &e)?)),
    }
}

fn intercept_read_bytes(
    state: &mut InterpreterState,
    args_start_reg: u16,
    _arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let path = extract_path_arg(state, args_start_reg, caller_base);
    if let Some(denied) = check_fs_permission(state, "read") {
        return Ok(Some(denied));
    }
    match std::fs::read(&path) {
        Ok(bytes) => {
            let list = alloc_byte_list(state, &bytes)?;
            Ok(Some(wrap_in_variant(state, "Result", 0, &[list])?))
        }
        Err(e) => Ok(Some(build_io_err(state, &e)?)),
    }
}

fn intercept_write_text(
    state: &mut InterpreterState,
    args_start_reg: u16,
    _arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let path = extract_path_arg(state, args_start_reg, caller_base);
    let contents = extract_text_arg(state, args_start_reg + 1, caller_base);
    if let Some(denied) = check_fs_permission(state, "write") {
        return Ok(Some(denied));
    }
    match std::fs::write(&path, contents.as_bytes()) {
        Ok(()) => {
            let unit = Value::unit();
            Ok(Some(wrap_in_variant(state, "Result", 0, &[unit])?))
        }
        Err(e) => Ok(Some(build_io_err(state, &e)?)),
    }
}

fn intercept_write_bytes(
    state: &mut InterpreterState,
    args_start_reg: u16,
    _arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let path = extract_path_arg(state, args_start_reg, caller_base);
    let bytes = extract_byte_list_arg(state, args_start_reg + 1, caller_base);
    if let Some(denied) = check_fs_permission(state, "write") {
        return Ok(Some(denied));
    }
    match std::fs::write(&path, &bytes) {
        Ok(()) => {
            let unit = Value::unit();
            Ok(Some(wrap_in_variant(state, "Result", 0, &[unit])?))
        }
        Err(e) => Ok(Some(build_io_err(state, &e)?)),
    }
}

fn intercept_exists(
    state: &mut InterpreterState,
    args_start_reg: u16,
    _arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let path = extract_path_arg(state, args_start_reg, caller_base);
    Ok(Some(Value::from_bool(std::path::Path::new(&path).exists())))
}

// ============================================================================
// Argument extraction
// ============================================================================

/// Extract a path argument, unwrapping CBGR refs and small/heap strings.
fn extract_path_arg(
    state: &InterpreterState,
    reg: u16,
    caller_base: u32,
) -> String {
    extract_text_arg(state, reg, caller_base)
}

/// Extract a Text argument from a register, transparently handling
/// CBGR-style register references (`&text` → negative-int encoding).
fn extract_text_arg(state: &InterpreterState, reg: u16, caller_base: u32) -> String {
    let v = state.registers.get(caller_base, crate::instruction::Reg(reg));
    let unwrapped = if super::cbgr_helpers::is_cbgr_ref(&v) {
        let (abs_index, _) = super::cbgr_helpers::decode_cbgr_ref(v.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        v
    };
    extract_string(&unwrapped, state)
}

/// Extract a `&[Byte]` (or `List<Byte>`) argument from a register
/// into an owned `Vec<u8>`.  Reads the List header `[len, cap,
/// backing_ptr]` and copies the byte payload.
fn extract_byte_list_arg(state: &InterpreterState, reg: u16, caller_base: u32) -> Vec<u8> {
    let v = state.registers.get(caller_base, crate::instruction::Reg(reg));
    let unwrapped = if super::cbgr_helpers::is_cbgr_ref(&v) {
        let (abs_index, _) = super::cbgr_helpers::decode_cbgr_ref(v.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        v
    };
    if !unwrapped.is_ptr() || unwrapped.is_nil() {
        return Vec::new();
    }
    let base = unwrapped.as_ptr::<u8>();
    if base.is_null() {
        return Vec::new();
    }
    unsafe {
        let header = &*(base as *const heap::ObjectHeader);
        if header.type_id != TypeId::LIST {
            return Vec::new();
        }
        let data_ptr = base.add(heap::OBJECT_HEADER_SIZE) as *const Value;
        let len = (*data_ptr).as_i64() as usize;
        let backing_val = *data_ptr.add(2);
        if !backing_val.is_ptr() || backing_val.is_nil() {
            return Vec::new();
        }
        let backing_ptr = backing_val.as_ptr::<u8>();
        if backing_ptr.is_null() {
            return Vec::new();
        }
        let backing_data = backing_ptr.add(heap::OBJECT_HEADER_SIZE);
        let slice = std::slice::from_raw_parts(backing_data, len);
        slice.to_vec()
    }
}

// ============================================================================
// Permission gate
// ============================================================================

/// Check that the script has FileSystem permission for the given
/// access kind (`"read"` or `"write"`).  Returns `Some(denied_err)`
/// when blocked — the caller substitutes the value into `dst` and
/// short-circuits.  Returns `None` when allowed.
fn check_fs_permission(state: &mut InterpreterState, _kind: &str) -> Option<Value> {
    if state.check_permission(PermissionScope::FileSystem, 0) == PermissionDecision::Deny {
        // Build an Err(PermissionDenied) result.
        let kind_variant = build_io_error_kind(state, "PermissionDenied", 1).ok()?;
        let msg_text = alloc_string_value(state, "permission denied: filesystem access").ok()?;
        let msg_some = wrap_in_variant(state, "Maybe", 1, &[msg_text]).ok()?;
        let stream_err = alloc_record_n_fields(state, "StreamError", &[kind_variant, msg_some]).ok()?;
        return wrap_in_variant(state, "Result", 1, &[stream_err]).ok();
    }
    None
}

// ============================================================================
// Result/StreamError construction
// ============================================================================

fn build_io_err(state: &mut InterpreterState, e: &std::io::Error) -> InterpreterResult<Value> {
    use std::io::ErrorKind as K;
    let (kind_name, kind_tag) = match e.kind() {
        K::NotFound          => ("NotFound", 0u32),
        K::PermissionDenied  => ("PermissionDenied", 1),
        K::ConnectionRefused => ("ConnectionRefused", 2),
        K::ConnectionReset   => ("ConnectionReset", 3),
        K::ConnectionAborted => ("ConnectionAborted", 4),
        K::NotConnected      => ("NotConnected", 5),
        K::AddrInUse         => ("AddrInUse", 6),
        K::AddrNotAvailable  => ("AddrNotAvailable", 7),
        K::BrokenPipe        => ("BrokenPipe", 8),
        K::AlreadyExists     => ("AlreadyExists", 9),
        K::WouldBlock        => ("WouldBlock", 10),
        K::InvalidInput      => ("InvalidInput", 11),
        K::InvalidData       => ("InvalidData", 12),
        K::TimedOut          => ("TimedOut", 13),
        K::WriteZero         => ("WriteZero", 14),
        K::Interrupted       => ("Interrupted", 15),
        K::UnexpectedEof     => ("UnexpectedEof", 16),
        K::OutOfMemory       => ("OutOfMemory", 17),
        K::Unsupported       => ("Unsupported", 18),
        _                    => ("Other", 19),
    };
    let kind_variant = build_io_error_kind(state, kind_name, kind_tag)?;
    let msg_text = alloc_string_value(state, &format!("{}", e))?;
    let msg_some = wrap_in_variant(state, "Maybe", 1, &[msg_text])?;
    let stream_err = alloc_record_n_fields(state, "StreamError", &[kind_variant, msg_some])?;
    wrap_in_variant(state, "Result", 1, &[stream_err])
}

fn build_io_error_kind(
    state: &mut InterpreterState,
    _name: &str,
    tag: u32,
) -> InterpreterResult<Value> {
    // IoErrorKind is a unit-only sum type — variant has no payload.
    wrap_in_variant(state, "IoErrorKind", tag, &[])
}

// ============================================================================
// Heap helpers (mirror shell_runtime.rs — kept private to avoid
// cross-module coupling; same layout contract).
// ============================================================================

fn alloc_byte_list(state: &mut InterpreterState, bytes: &[u8]) -> InterpreterResult<Value> {
    use crate::interpreter::heap::OBJECT_HEADER_SIZE;
    let len = bytes.len();
    let cap = if len < 16 { 16 } else { len };
    let backing = state.heap.alloc(TypeId::LIST, cap)?;
    state.record_allocation();
    if !bytes.is_empty() {
        let backing_data = unsafe { (backing.as_ptr() as *mut u8).add(OBJECT_HEADER_SIZE) };
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), backing_data, len); }
    }
    let list = state.heap.alloc(TypeId::LIST, 3 * std::mem::size_of::<Value>())?;
    state.record_allocation();
    let data_ptr = unsafe { (list.as_ptr() as *mut u8).add(OBJECT_HEADER_SIZE) as *mut Value };
    unsafe {
        *data_ptr = Value::from_i64(len as i64);
        *data_ptr.add(1) = Value::from_i64(cap as i64);
        *data_ptr.add(2) = Value::from_ptr(backing.as_ptr() as *mut u8);
    }
    Ok(Value::from_ptr(list.as_ptr() as *mut u8))
}

fn alloc_record_n_fields(
    state: &mut InterpreterState,
    type_name: &str,
    fields: &[Value],
) -> InterpreterResult<Value> {
    use crate::interpreter::heap::OBJECT_HEADER_SIZE;
    let type_id = lookup_type_id_by_name(state, type_name).unwrap_or(TypeId(0x9000));
    let payload_size = fields.len() * std::mem::size_of::<Value>();
    let obj = state.heap.alloc(type_id, payload_size)?;
    state.record_allocation();
    let data_ptr = unsafe { (obj.as_ptr() as *mut u8).add(OBJECT_HEADER_SIZE) as *mut Value };
    for (i, v) in fields.iter().enumerate() {
        unsafe { *data_ptr.add(i) = *v; }
    }
    Ok(Value::from_ptr(obj.as_ptr() as *mut u8))
}

fn wrap_in_variant(
    state: &mut InterpreterState,
    type_name: &str,
    tag: u32,
    fields: &[Value],
) -> InterpreterResult<Value> {
    use crate::interpreter::heap::OBJECT_HEADER_SIZE;
    let type_id = lookup_type_id_by_name(state, type_name).unwrap_or(TypeId(0x8000 + tag));
    let field_count = fields.len() as u32;
    let data_size = 8 + (fields.len() * std::mem::size_of::<Value>());
    let obj = state.heap.alloc(type_id, data_size)?;
    state.record_allocation();
    let base = obj.as_ptr() as *mut u8;
    unsafe {
        let tag_ptr = base.add(OBJECT_HEADER_SIZE) as *mut u32;
        *tag_ptr = tag;
        *tag_ptr.add(1) = field_count;
        let payload_ptr = base.add(OBJECT_HEADER_SIZE + 8) as *mut Value;
        for (i, v) in fields.iter().enumerate() {
            *payload_ptr.add(i) = *v;
        }
    }
    Ok(Value::from_ptr(base))
}

fn lookup_type_id_by_name(state: &InterpreterState, name: &str) -> Option<TypeId> {
    state
        .module
        .types
        .iter()
        .find(|td| {
            state.module.strings.get(td.name) == Some(name)
                && !matches!(td.kind, crate::types::TypeKind::Protocol)
        })
        .map(|td| td.id)
}
