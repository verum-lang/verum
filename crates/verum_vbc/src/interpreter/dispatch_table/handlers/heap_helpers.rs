//! Shared heap-marshaling helpers for the Tier-0 intercept modules
//! (`shell_runtime`, `file_runtime`, `env_runtime`, `stdio_runtime`,
//! `process_runtime`, `net_runtime`).
//!

//! Pre-extraction, every sibling module carried a verbatim copy of
//! these primitives — `alloc_byte_list`, `alloc_record_n_fields`,
//! `wrap_in_variant`, `lookup_type_id_by_name`, `extract_text_arg`,
//! `extract_byte_slice`, `read_buffer_capacity`, `write_into_byte_slice`.
//! ~500 lines duplicated across six files, with a real history of
//! drift: the `alloc_byte_list` Value-per-element layout fix had to
//! be applied THREE times after the same defect was originally
//! introduced in three modules.
//!

//! This module is the single canonical source. Sibling modules
//! `use super::heap_helpers::{...}` — adding a new intercept module
//! costs zero copy-paste of these primitives.
//!

//! # Layout invariants this module encodes
//!

//!  * **Variant heap-record**: `[ObjectHeader][tag:u32][n_fields:u32]
//!  [Value;N]`. Constructed via [`wrap_in_variant`].
//!  * **Plain record**: `[ObjectHeader][Value;N]`. Constructed via
//!  [`alloc_record_n_fields`].
//!  * **`List<T>` triple-header**: `[len:Value][cap:Value][backing:Value]`
//!  where backing is `[ObjectHeader][Value;cap]` — one Value-slot
//!  per element regardless of T. `bytes[i]` reads via
//!  `*(backing as *const Value).add(i)` (see
//!  `memory_collections::handle_get_index` LIST arm). Constructed
//!  via [`alloc_byte_list`]; reverse-decoded via
//!  [`extract_byte_slice`].
//!

//! Any change to these layouts must update BOTH this module AND the
//! `memory_collections` GetE/SetE handlers in lockstep.

use super::super::super::error::InterpreterResult;
use super::cbgr_helpers::{decode_cbgr_ref, is_cbgr_ref};
use super::string_helpers::extract_string;
use crate::interpreter::heap;
use crate::interpreter::state::InterpreterState;
use crate::types::TypeId;
use crate::value::Value;

// ============================================================================
// TypeId resolution
// ============================================================================

/// Resolve a stdlib type name (e.g. `"Result"`, `"TcpStream"`) to its
/// runtime [`TypeId`] in the loaded module's type table. Filters out
/// `TypeKind::Protocol` to avoid name collisions with the protocol
/// declarations (the impl-typed records are what we want to allocate).
///

/// Returns `None` when the type isn't loaded — caller falls back to
/// the synthetic-id pattern (`TypeId(0x9000)` for records,
/// `TypeId(0x8000 + tag)` for variants) which the interpreter accepts
/// at a slight semantic cost.
pub(super) fn lookup_type_id_by_name(state: &InterpreterState, name: &str) -> Option<TypeId> {
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

// ============================================================================
// Record / variant allocation
// ============================================================================

/// Allocate a heap record with N field slots — `[ObjectHeader][Value;N]`.
/// Used for stdlib record types like `Output { status, stdout_bytes,
/// stderr_bytes }`, `TcpStream { fd, peer_addr }`, etc.
pub(super) fn alloc_record_n_fields(
    state: &mut InterpreterState,
    type_name: &str,
    fields: &[Value],
) -> InterpreterResult<Value> {
    use heap::OBJECT_HEADER_SIZE;
    let type_id = lookup_type_id_by_name(state, type_name).unwrap_or(TypeId(0x9000));
    let payload_size = fields.len() * std::mem::size_of::<Value>();
    let obj = state.heap.alloc(type_id, payload_size)?;
    state.record_allocation();
    let data_ptr = unsafe { (obj.as_ptr() as *mut u8).add(OBJECT_HEADER_SIZE) as *mut Value };
    for (i, v) in fields.iter().enumerate() {
        unsafe {
            *data_ptr.add(i) = *v;
        }
    }
    Ok(Value::from_ptr(obj.as_ptr() as *mut u8))
}

/// Allocate a heap variant of a sum type — `[ObjectHeader][tag:u32]
/// [n_fields:u32][Value;N]`. `tag` is the variant index in the sum
/// type's declaration order (e.g. `Result.Ok = 0`, `Result.Err = 1`).
pub(super) fn wrap_in_variant(
    state: &mut InterpreterState,
    type_name: &str,
    tag: u32,
    fields: &[Value],
) -> InterpreterResult<Value> {
    use heap::OBJECT_HEADER_SIZE;
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

// ============================================================================
// Byte-list construction + decoding (List<Byte>)
// ============================================================================

/// Allocate a `List<Byte>` heap value from a Rust byte slice.
///

/// **Layout** — three-Value header `[len, cap, backing_ptr]` where
/// backing is one Value-slot per element (each byte boxed as
/// `Value::from_i64(b as i64)`). Matches the canonical List<T>
/// shape from `method_dispatch::handle_call_method`'s empty-List
/// path so script-side `bytes[i]` reads the actual byte rather than
/// header bits.
pub(super) fn alloc_byte_list(
    state: &mut InterpreterState,
    bytes: &[u8],
) -> InterpreterResult<Value> {
    use heap::OBJECT_HEADER_SIZE;
    let len = bytes.len();
    let cap = if len < 16 { 16 } else { len };
    let backing = state.heap.alloc_array(TypeId::LIST, cap)?;
    state.record_allocation();
    let backing_data =
        unsafe { (backing.as_ptr() as *mut u8).add(OBJECT_HEADER_SIZE) as *mut Value };
    for (i, b) in bytes.iter().enumerate() {
        unsafe {
            *backing_data.add(i) = Value::from_i64(*b as i64);
        }
    }
    let list = state
        .heap
        .alloc(TypeId::LIST, 3 * std::mem::size_of::<Value>())?;
    state.record_allocation();
    let data_ptr = unsafe { (list.as_ptr() as *mut u8).add(OBJECT_HEADER_SIZE) as *mut Value };
    unsafe {
        *data_ptr = Value::from_i64(len as i64);
        *data_ptr.add(1) = Value::from_i64(cap as i64);
        *data_ptr.add(2) = Value::from_ptr(backing.as_ptr() as *mut u8);
    }
    Ok(Value::from_ptr(list.as_ptr() as *mut u8))
}

/// Allocate a `List<Byte>` heap value with **packed-byte backing** —
/// 1 byte per element instead of one NaN-boxed Value (8 bytes) per
/// element.  Closes red-team §4: at 10K connections × 16-KiB read
/// buffer, this drops the steady-state heap from 1.28 GiB to 160 MiB.
///
/// **Layout** — three-Value header `[len, cap, backing_ptr]` tagged
/// with `TypeId::BYTE_LIST`; `backing` is a contiguous `[u8; cap]`
/// region in heap-managed memory.  The list-shaped read primitives
/// (`extract_byte_slice`, `read_buffer_capacity`, `write_into_byte_slice`)
/// dispatch on the header's `TypeId` to walk either the canonical
/// Value-per-element backing (`LIST`) or this packed backing
/// (`BYTE_LIST`).
///
/// **Mutation discipline** — this allocator is appropriate when the
/// caller knows the result is consumed read-only (intrinsic results:
/// stdout, stderr, file contents).  Script-side `.push()` /
/// `.pop()` / `.set()` against a `BYTE_LIST` requires the
/// matching writer-handler migration (tracked separately) — until
/// that lands, prefer [`alloc_byte_list`] for mutable byte lists.
#[allow(dead_code)]
pub(super) fn alloc_byte_list_packed(
    state: &mut InterpreterState,
    bytes: &[u8],
) -> InterpreterResult<Value> {
    use heap::OBJECT_HEADER_SIZE;
    let len = bytes.len();
    let cap = if len < 16 { 16 } else { len };
    let backing = state.heap.alloc(TypeId::BYTE_LIST, cap)?;
    state.record_allocation();
    let backing_data = unsafe { (backing.as_ptr() as *mut u8).add(OBJECT_HEADER_SIZE) };
    if !bytes.is_empty() {
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), backing_data, len);
        }
    }
    let list = state
        .heap
        .alloc(TypeId::BYTE_LIST, 3 * std::mem::size_of::<Value>())?;
    state.record_allocation();
    let data_ptr = unsafe { (list.as_ptr() as *mut u8).add(OBJECT_HEADER_SIZE) as *mut Value };
    unsafe {
        *data_ptr = Value::from_i64(len as i64);
        *data_ptr.add(1) = Value::from_i64(cap as i64);
        *data_ptr.add(2) = Value::from_ptr(backing.as_ptr() as *mut u8);
    }
    Ok(Value::from_ptr(list.as_ptr() as *mut u8))
}

/// Decode a `&[Byte]` argument into an owned `Vec<u8>`. Walks
/// either a FatRef-shaped slice (elem_size 0 = NaN-boxed Values, 1
/// = packed bytes) or a `List<Byte>` Value-per-element backing.
/// Empty Vec when the value isn't a byte container.
pub(super) fn extract_byte_slice(state: &InterpreterState, reg: u16, caller_base: u32) -> Vec<u8> {
    let v = state
        .registers
        .get(caller_base, crate::instruction::Reg(reg));
    let unwrapped = if is_cbgr_ref(&v) {
        let (abs_index, _) = decode_cbgr_ref(v.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        v
    };
    if unwrapped.is_fat_ref() {
        let fr = unwrapped.as_fat_ref();
        let len = fr.len() as usize;
        if fr.ptr().is_null() || len == 0 {
            return Vec::new();
        }
        return match fr.reserved {
            1 => unsafe { std::slice::from_raw_parts(fr.ptr(), len) }.to_vec(),
            _ => {
                let mut out = Vec::with_capacity(len);
                for i in 0..len {
                    let elem = unsafe { *(fr.ptr() as *const Value).add(i) };
                    out.push(elem.as_i64() as u8);
                }
                out
            }
        };
    }
    if unwrapped.is_ptr() && !unwrapped.is_nil() {
        let ptr = unwrapped.as_ptr::<u8>();
        if ptr.is_null() {
            return Vec::new();
        }
        let header = unsafe { &*(ptr as *const heap::ObjectHeader) };
        if header.type_id == TypeId::LIST {
            let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
            let len = unsafe { (*data_ptr).as_i64() } as usize;
            let backing_v = unsafe { *data_ptr.add(2) };
            if backing_v.is_ptr() && !backing_v.is_nil() {
                let backing = backing_v.as_ptr::<u8>();
                if !backing.is_null() {
                    let backing_data =
                        unsafe { backing.add(heap::OBJECT_HEADER_SIZE) as *const Value };
                    let mut out = Vec::with_capacity(len);
                    for i in 0..len {
                        out.push(unsafe { (*backing_data.add(i)).as_i64() } as u8);
                    }
                    return out;
                }
            }
        }
        if header.type_id == TypeId::BYTE_LIST {
            // Packed-byte backing — `[u8; cap]` after the backing
            // ObjectHeader.  One byte per element; no Value boxing.
            let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
            let len = unsafe { (*data_ptr).as_i64() } as usize;
            let backing_v = unsafe { *data_ptr.add(2) };
            if backing_v.is_ptr() && !backing_v.is_nil() {
                let backing = backing_v.as_ptr::<u8>();
                if !backing.is_null() && len > 0 {
                    let backing_data = unsafe { backing.add(heap::OBJECT_HEADER_SIZE) };
                    return unsafe { std::slice::from_raw_parts(backing_data, len) }.to_vec();
                }
                return Vec::new();
            }
        }
    }
    _ = state; // suppress unused warning when no extract paths fire.
    Vec::new()
}

/// Capacity of a byte buffer for sizing `recv()` — the slice's
/// declared length when it's a FatRef, the list's len when it's
/// `List<Byte>` (canonical or packed-byte layout). Returns None
/// when the shape isn't recognised.
pub(super) fn read_buffer_capacity(v: Value) -> Option<usize> {
    if v.is_fat_ref() {
        return Some(v.as_fat_ref().len() as usize);
    }
    if v.is_ptr() && !v.is_nil() {
        let ptr = v.as_ptr::<u8>();
        if ptr.is_null() {
            return None;
        }
        let header = unsafe { &*(ptr as *const heap::ObjectHeader) };
        if header.type_id.is_list_like() {
            // Both `LIST` and `BYTE_LIST` share the 3-Value header
            // shape `[len, cap, backing_ptr]`; the layout difference
            // is in the backing only.  `len` (slot 0) is the same
            // across both.
            let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
            return Some(unsafe { (*data_ptr).as_i64() } as usize);
        }
    }
    None
}

/// Write `bytes` into the backing of a `&mut [Byte]`-shaped value.
/// Best-effort no-op when the shape isn't recognised — the script's
/// `Ok(n)` arm conveys the byte count regardless, so a partial
/// write doesn't corrupt the caller's view of how many bytes
/// arrived. Updates the List header's `len` field to reflect
/// bytes actually written (so the script's subsequent `.len()` /
/// `.iter()` see the new logical length).
pub(super) fn write_into_byte_slice(v: Value, bytes: &[u8]) {
    if v.is_fat_ref() {
        let fr = v.as_fat_ref();
        let cap = fr.len() as usize;
        let n = bytes.len().min(cap);
        if fr.ptr().is_null() || n == 0 {
            return;
        }
        match fr.reserved {
            1 => unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), fr.ptr(), n);
            },
            _ => {
                let dst = fr.ptr() as *mut Value;
                for i in 0..n {
                    unsafe { *dst.add(i) = Value::from_i64(bytes[i] as i64) };
                }
            }
        }
        return;
    }
    if v.is_ptr() && !v.is_nil() {
        let ptr = v.as_ptr::<u8>();
        if ptr.is_null() {
            return;
        }
        let header = unsafe { &*(ptr as *const heap::ObjectHeader) };
        if header.type_id == TypeId::LIST {
            let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value };
            let cap = unsafe { (*data_ptr.add(1)).as_i64() } as usize;
            let n = bytes.len().min(cap);
            let backing_v = unsafe { *data_ptr.add(2) };
            if backing_v.is_ptr() && !backing_v.is_nil() {
                let backing = backing_v.as_ptr::<u8>();
                if !backing.is_null() {
                    let backing_data =
                        unsafe { backing.add(heap::OBJECT_HEADER_SIZE) as *mut Value };
                    for i in 0..n {
                        unsafe {
                            *backing_data.add(i) = Value::from_i64(bytes[i] as i64);
                        }
                    }
                    unsafe {
                        *data_ptr = Value::from_i64(n as i64);
                    }
                }
            }
            return;
        }
        if header.type_id == TypeId::BYTE_LIST {
            // Packed-byte backing: write bytes directly via memcpy.
            let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value };
            let cap = unsafe { (*data_ptr.add(1)).as_i64() } as usize;
            let n = bytes.len().min(cap);
            let backing_v = unsafe { *data_ptr.add(2) };
            if backing_v.is_ptr() && !backing_v.is_nil() {
                let backing = backing_v.as_ptr::<u8>();
                if !backing.is_null() && n > 0 {
                    let backing_data = unsafe { backing.add(heap::OBJECT_HEADER_SIZE) };
                    unsafe {
                        std::ptr::copy_nonoverlapping(bytes.as_ptr(), backing_data, n);
                        *data_ptr = Value::from_i64(n as i64);
                    }
                }
            }
        }
    }
}

// ============================================================================
// Text argument extraction
// ============================================================================

/// Extract a `&Text` argument from a register, transparently
/// handling CBGR-style register references (negative-int encoding
/// of `&self`/`&mut self`) and thin-ref auto-deref (heap-pointer-
/// to-Value).
pub(super) fn extract_text_arg(state: &InterpreterState, reg: u16, caller_base: u32) -> String {
    let v = state
        .registers
        .get(caller_base, crate::instruction::Reg(reg));
    let mut unwrapped = if is_cbgr_ref(&v) {
        let (abs_index, _) = decode_cbgr_ref(v.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        v
    };
    if unwrapped.is_thin_ref() {
        let tr = unwrapped.as_thin_ref();
        if !tr.ptr.is_null() {
            unwrapped = unsafe { *(tr.ptr as *const Value) };
        }
    }
    extract_string(&unwrapped, state)
}

/// CBGR-or-thin-ref unwrap of a Value — the canonical first step
/// before any heap-shape probe of an `&self`/`&mut self` receiver.
pub(super) fn unwrap_ref(state: &InterpreterState, v: Value) -> Value {
    let mut cur = if is_cbgr_ref(&v) {
        let (abs_index, _) = decode_cbgr_ref(v.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        v
    };
    if cur.is_thin_ref() {
        let tr = cur.as_thin_ref();
        if !tr.ptr.is_null() {
            cur = unsafe { *(tr.ptr as *const Value) };
        }
    }
    cur
}

// ============================================================================
// Receiver-shape probes
// ============================================================================

/// Generic shape probe — returns true iff `v` is a heap pointer to
/// an object whose TypeId resolves (in the loaded module's type
/// table) to a type named `name`. Used by the method-dispatch
/// hooks to gate their intercepts on the receiver actually being
/// the expected stdlib type.
pub(super) fn is_record_typed_as(state: &InterpreterState, v: Value, name: &str) -> bool {
    if !v.is_ptr() || v.is_nil() {
        return false;
    }
    let ptr = v.as_ptr::<u8>();
    if ptr.is_null() {
        return false;
    }
    let header = unsafe { &*(ptr as *const heap::ObjectHeader) };
    state
        .module
        .types
        .iter()
        .any(|td| td.id == header.type_id && state.module.strings.get(td.name) == Some(name))
}
