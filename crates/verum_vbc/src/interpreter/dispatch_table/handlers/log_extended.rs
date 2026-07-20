//! Log and memory extended opcode handlers for VBC interpreter dispatch.

use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;
use super::envelope::dispatch_enveloped;
use crate::instruction::{LogSubOpcode, Opcode};
use crate::value::Value;
// LIST-REALLOC-CANONICAL-1: realloc must recognise interpreter-heap backing
// objects (NewList/ListPush arrays) vs opaque std::alloc buffers.
use super::super::super::heap;
use crate::types::TypeId;

/// LogExtended (0xBE) - Structured logging operations.
///

/// Sub-opcodes organized by category:
/// - 0x00-0x04: Log levels (Info, Warning, Error, Debug, Trace)
/// - 0x10: Structured logging with key-value pairs
/// - 0x20-0x22: Control operations (Flush, SetLevel, GetLevel)
///

/// # Performance
///

/// Logging is inherently I/O-bound, so the runtime overhead (~50ns)
/// is negligible compared to actual I/O operations.
///

/// Extended logging opcode (0xCB + sub-opcode): structured logging with levels (Debug, Info,
/// Warn, Error, Fatal), structured fields, and context integration. ~50ns runtime overhead
/// is negligible vs I/O cost.
pub(in super::super) fn handle_log_extended(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    dispatch_enveloped(state, log_extended_body)
}

/// `LogExtended` sub-op arms. Invoked through
/// [`dispatch_enveloped`](super::envelope::dispatch_enveloped), which owns the
/// sub-op byte, the operand-length envelope and the pc reposition — an arm may
/// read any number of operands, and may `return` early, without desynchronising
/// the instruction stream.
fn log_extended_body(
    state: &mut InterpreterState,
    sub_op_byte: u8,
) -> InterpreterResult<DispatchResult> {
    let sub_op = LogSubOpcode::from_byte(sub_op_byte);

    match sub_op {
        // ================================================================
        // Log Levels (0x00-0x04)
        //
        // Wire is [dst][msg]. The canonical `encode_operands` helper
        // prefixes the destination register unconditionally — even for
        // void-returning sub-ops like these, which never write it. Reading
        // `msg` first therefore consumed the DST byte and logged the
        // uninitialised destination temp instead of the message, leaving
        // the real operand unread (T0418).
        // ================================================================
        Some(LogSubOpcode::Info) => {
            let _dst = read_reg(state)?;
            let msg_reg = read_reg(state)?;
            let msg = state.get_reg(msg_reg);
            let msg_str = format_value_for_log(&msg);
            // Use eprintln for now; in production, this would use the log crate
            eprintln!("[INFO] {}", msg_str);
            Ok(DispatchResult::Continue)
        }

        Some(LogSubOpcode::Warning) => {
            let _dst = read_reg(state)?;
            let msg_reg = read_reg(state)?;
            let msg = state.get_reg(msg_reg);
            let msg_str = format_value_for_log(&msg);
            eprintln!("[WARN] {}", msg_str);
            Ok(DispatchResult::Continue)
        }

        Some(LogSubOpcode::Error) => {
            let _dst = read_reg(state)?;
            let msg_reg = read_reg(state)?;
            let msg = state.get_reg(msg_reg);
            let msg_str = format_value_for_log(&msg);
            eprintln!("[ERROR] {}", msg_str);
            Ok(DispatchResult::Continue)
        }

        Some(LogSubOpcode::Debug) => {
            let _dst = read_reg(state)?;
            let msg_reg = read_reg(state)?;
            let msg = state.get_reg(msg_reg);
            if state.log_level >= 3 {
                let msg_str = format_value_for_log(&msg);
                eprintln!("[DEBUG] {}", msg_str);
            }
            Ok(DispatchResult::Continue)
        }

        Some(LogSubOpcode::Trace) => {
            let _dst = read_reg(state)?;
            let msg_reg = read_reg(state)?;
            let msg = state.get_reg(msg_reg);
            if state.log_level >= 4 {
                let msg_str = format_value_for_log(&msg);
                eprintln!("[TRACE] {}", msg_str);
            }
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Structured Logging (0x10)
        // ================================================================
        Some(LogSubOpcode::Structured) => {
            let _level = read_u8(state)?;
            let msg_reg = read_reg(state)?;
            let _kvs_reg = read_reg(state)?;
            // For now, just log the message; full structured logging TBD
            let msg = state.get_reg(msg_reg);
            let msg_str = format_value_for_log(&msg);
            eprintln!("[STRUCTURED] {}", msg_str);
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Control Operations (0x20-0x22)
        // ================================================================
        Some(LogSubOpcode::Flush) => {
            // Flush stderr (logs go to stderr)
            use std::io::Write;
            let _ = std::io::stderr().flush();
            Ok(DispatchResult::Continue)
        }

        Some(LogSubOpcode::SetLevel) => {
            let level_reg = read_reg(state)?;
            let level = state.get_reg(level_reg).as_i64();
            state.log_level = level.clamp(0, 4);
            Ok(DispatchResult::Continue)
        }

        Some(LogSubOpcode::GetLevel) => {
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(state.log_level));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Unimplemented sub-opcodes
        // ================================================================
        None => Err(InterpreterError::NotImplemented {
            feature: "log_extended sub-opcode",
            opcode: Some(Opcode::LogExtended),
        }),
    }
}

/// MemExtended (0xBF) - Memory allocation operations.
///

/// Sub-opcodes:
/// - 0x00: Alloc - allocate heap memory
/// - 0x01: AllocZeroed - allocate zeroed heap memory
/// - 0x02: Dealloc - deallocate heap memory
/// - 0x03: Realloc - reallocate heap memory
/// - 0x04: Swap - swap two values in place
/// - 0x05: Replace - replace value and return old
/// - 0x06: NewByteList - allocate a `List<Byte>` with packed-byte
///   backing (1 byte/element, vs 8 for canonical `NewList`).
///   Closes red-team §4 runtime memory amplification: 10K connections
///   × 16-KiB read buffer drops from 1.28 GiB to 160 MiB.
///   Format: `[dst:reg, cap:reg]`.
pub(in super::super) fn handle_mem_extended(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    dispatch_enveloped(state, mem_extended_body)
}

/// `MemExtended` sub-op arms. Invoked through
/// [`dispatch_enveloped`](super::envelope::dispatch_enveloped), which owns the
/// sub-op byte, the operand-length envelope and the pc reposition.
///
/// This family is the canonical instance of the defect the envelope exists to
/// kill, and the reason the authority is unconditional. Each arm reads the
/// register count of the **registry's** declared param shape (`AllocZeroed`
/// reads dst + size + align = 3), but a Verum-source forward declaration may
/// bind a SUBSET of those params — `core/intrinsics/runtime/os.vr`'s
/// `__alloc_zeroed_raw(size: Int)` is annotated `@intrinsic("alloc_zeroed")`
/// with one argument while the registry declares two. Codegen then emits FEWER
/// operand bytes than the arm reads, the arm's `read_reg` overshoots into the
/// next instruction's opcode byte, and the pc stays misaligned for the rest of
/// the function: `GenerationalArena.new(N)` surfaced this as a "Null pointer
/// dereference" at a downstream `SetF` whose object register had become
/// garbage.
///
/// Arms may therefore read any number of bytes, in any order, and may `return`
/// early — the envelope re-establishes the instruction boundary afterwards, so
/// codegen drift can no longer leak past it.
fn mem_extended_body(
    state: &mut InterpreterState,
    sub_op: u8,
) -> InterpreterResult<DispatchResult> {
    match sub_op {
        // Alloc: [dst, size, align]
        0x00 => {
            let dst = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let _align_reg = read_reg(state)?;

            let size = state.get_reg(size_reg).as_i64() as usize;

            // Allocate memory using system allocator
            let layout = std::alloc::Layout::from_size_align(size.max(1), 8).map_err(|_| {
                InterpreterError::Panic {
                    message: "invalid allocation layout".into(),
                }
            })?;
            let ptr = unsafe { std::alloc::alloc(layout) };
            if ptr.is_null() {
                return Err(InterpreterError::Panic {
                    message: "allocation failed".into(),
                });
            }

            state.set_reg(dst, Value::from_ptr(ptr as *mut ()));
            Ok(DispatchResult::Continue)
        }

        // AllocZeroed: [dst, size, align]
        0x01 => {
            let dst = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let _align_reg = read_reg(state)?;

            let size = state.get_reg(size_reg).as_i64() as usize;

            // Allocate zeroed memory using system allocator
            let layout = std::alloc::Layout::from_size_align(size.max(1), 8).map_err(|_| {
                InterpreterError::Panic {
                    message: "invalid allocation layout".into(),
                }
            })?;
            let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
            if ptr.is_null() {
                return Err(InterpreterError::Panic {
                    message: "allocation failed".into(),
                });
            }

            state.set_reg(dst, Value::from_ptr(ptr as *mut ()));
            Ok(DispatchResult::Continue)
        }

        // Dealloc: [ptr, size, align]
        0x02 => {
            let ptr_reg = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let _align_reg = read_reg(state)?;

            let ptr = state.get_reg(ptr_reg).as_ptr::<u8>();
            let size = state.get_reg(size_reg).as_i64() as usize;

            if !ptr.is_null() && size > 0 {
                let layout = std::alloc::Layout::from_size_align(size, 8).map_err(|_| {
                    InterpreterError::Panic {
                        message: "invalid deallocation layout".into(),
                    }
                })?;
                unsafe { std::alloc::dealloc(ptr, layout) };
            }

            Ok(DispatchResult::Continue)
        }

        // Realloc: [dst, ptr, old_size, new_size, align]
        0x03 => {
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let old_size_reg = read_reg(state)?;
            let new_size_reg = read_reg(state)?;
            let align_reg = read_reg(state)?;

            let ptr = state.get_reg(ptr_reg).as_ptr::<u8>();
            let old_size = state.get_reg(old_size_reg).as_i64() as usize;
            let new_size = state.get_reg(new_size_reg).as_i64() as usize;
            let align = {
                let a = state.get_reg(align_reg).as_i64() as usize;
                if a == 0 { 8 } else { a }
            };

            // LIST-REALLOC-CANONICAL-1: when `ptr` is an interpreter-heap
            // object, the .vr `resize_buffer`'s realloc reached a CANONICAL
            // collection backing (a NewList/ListPush array with an
            // ObjectHeader + Value/byte slots), NOT a std::alloc buffer.
            // `List.new`/`with_capacity`/`push`/`get` are intercepted onto
            // `state.heap`, but `reserve`/`resize` run the .vr body which
            // funnels through this realloc. Using std::alloc here freed the
            // heap-object pointer via `std::alloc::dealloc` (an address the
            // system allocator never returned -> heap corruption -> SIGABRT at
            // drop) and copied from the header offset (data loss). Grow via
            // `state.heap` exactly like `handle_list_push`: allocate a new
            // backing, copy the data region at +OBJECT_HEADER_SIZE, and leave
            // the old backing for the GC (never std::alloc::dealloc it).
            if !ptr.is_null() && state.heap.contains(ptr as *const heap::ObjectHeader) {
                let is_byte = {
                    let header = unsafe { heap::ObjectHeader::ref_or_stub(ptr) };
                    header.type_id == TypeId::BYTE_LIST
                };
                let elem_size = if is_byte {
                    1usize
                } else {
                    std::mem::size_of::<Value>()
                };
                let new_cap_slots = new_size / elem_size.max(1);
                let new_backing = if is_byte {
                    state.heap.alloc(TypeId::BYTE_LIST, new_cap_slots)?
                } else {
                    state.heap.alloc_array(TypeId::UNIT, new_cap_slots)?
                };
                state.record_allocation();
                let new_ptr = new_backing.as_ptr() as *mut u8;
                let copy_bytes = old_size.min(new_size);
                if copy_bytes > 0 {
                    let old_data = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) };
                    let new_data = unsafe { new_ptr.add(heap::OBJECT_HEADER_SIZE) };
                    unsafe {
                        std::ptr::copy_nonoverlapping(old_data, new_data, copy_bytes);
                    }
                }
                state.set_reg(dst, Value::from_ptr(new_ptr as *mut ()));
                // T0429 — this early return is safe BY CONSTRUCTION, and the
                // reason is structural, not local: the pc reposition lives in
                // `dispatch_enveloped`, our CALLER, so returning from this
                // function cannot bypass it. When the correction still lived at
                // the tail of this handler, this exact `return` skipped it and
                // re-opened the desync the handler was written to close. Any
                // future fast path may return freely for the same reason —
                // just never reintroduce a pc fixup here.
                return Ok(DispatchResult::Continue);
            }

            let new_layout =
                std::alloc::Layout::from_size_align(new_size.max(1), align).map_err(|_| {
                    InterpreterError::Panic {
                        message: "invalid reallocation layout".into(),
                    }
                })?;
            let new_ptr = unsafe { std::alloc::alloc(new_layout) };
            if new_ptr.is_null() {
                return Err(InterpreterError::Panic {
                    message: "reallocation failed".into(),
                });
            }

            // Copy old data to new allocation (up to the smaller of old/new size)
            if !ptr.is_null() && old_size > 0 {
                let copy_size = old_size.min(new_size);
                unsafe { std::ptr::copy_nonoverlapping(ptr, new_ptr, copy_size) };
                // Zero the extra bytes if growing
                if new_size > old_size {
                    unsafe {
                        std::ptr::write_bytes(new_ptr.add(old_size), 0, new_size - old_size);
                    }
                }
                // Free old allocation
                if let Ok(old_layout) = std::alloc::Layout::from_size_align(old_size.max(1), align)
                {
                    unsafe { std::alloc::dealloc(ptr, old_layout) };
                }
            }

            state.set_reg(dst, Value::from_ptr(new_ptr as *mut ()));
            Ok(DispatchResult::Continue)
        }

        // Swap: [a, b]
        //
        // The args are `&mut T` references which materialise as CBGR
        // register-refs (negative-i64-encoded `(abs_index, generation)`
        // pairs) — NOT raw pointers.  Pre-fix `as_ptr::<u64>()` on a
        // CBGR ref dereferenced the negative integer as a pointer and
        // SIGSEGV'd at runtime.  Resolve through `cbgr_helpers` so the
        // swap operates on the abs-register slots in the Value world,
        // not raw memory addresses.
        0x04 => {
            use super::cbgr_helpers::{decode_cbgr_ref, is_cbgr_ref};
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a_val = state.get_reg(a_reg);
            let b_val = state.get_reg(b_reg);
            if is_cbgr_ref(&a_val) && is_cbgr_ref(&b_val) {
                let (a_abs, _) = decode_cbgr_ref(a_val);
                let (b_abs, _) = decode_cbgr_ref(b_val);
                let tmp = state.registers.get_absolute(a_abs);
                let b_inner = state.registers.get_absolute(b_abs);
                state.registers.set_absolute(a_abs, b_inner);
                state.registers.set_absolute(b_abs, tmp);
            } else {
                // Raw-pointer fallback (preserves the legacy path for
                // direct-pointer call sites that bypass CBGR encoding).
                let a_ptr = a_val.as_ptr::<u64>();
                let b_ptr = b_val.as_ptr::<u64>();
                if !a_ptr.is_null() && !b_ptr.is_null() {
                    unsafe { core::ptr::swap(a_ptr, b_ptr); }
                }
            }
            Ok(DispatchResult::Continue)
        }

        // Replace: [dst, dest, src]
        //
        // Same CBGR-ref handling as Swap above — `&mut T` materialises
        // as a CBGR register-ref, not a raw pointer.  Read the current
        // value out of the abs-register slot, store the new value, and
        // return the old value in `dst`.
        0x05 => {
            use super::cbgr_helpers::{decode_cbgr_ref, is_cbgr_ref};
            let dst = read_reg(state)?;
            let dest_reg = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let dest_val = state.get_reg(dest_reg);
            let src_val = state.get_reg(src_reg);
            if is_cbgr_ref(&dest_val) {
                let (abs, _) = decode_cbgr_ref(dest_val);
                let old = state.registers.get_absolute(abs);
                state.registers.set_absolute(abs, src_val);
                state.set_reg(dst, old);
            } else {
                // Raw-pointer fallback (legacy).
                let dest_ptr = dest_val.as_ptr::<u64>();
                if dest_ptr.is_null() {
                    return Err(InterpreterError::Panic {
                        message: "replace: null destination pointer".into(),
                    });
                }
                let new_val_u64 = src_val.as_i64() as u64;
                let old_val = unsafe { *dest_ptr };
                unsafe { *dest_ptr = new_val_u64 };
                state.set_reg(dst, Value::from_i64(old_val as i64));
            }
            Ok(DispatchResult::Continue)
        }

        // NewByteList: [dst, cap] — allocate `List<Byte>` with packed
        // 1-byte-per-element backing (TypeId::BYTE_LIST).  Mirrors
        // `handle_new_list`'s 3-Value-header layout but tags both the
        // list and its backing with `BYTE_LIST` and sizes the backing
        // as `cap` raw bytes rather than `cap * sizeof(Value)`.
        // Closes red-team §4 runtime memory half.
        0x06 => {
            use crate::interpreter::heap::OBJECT_HEADER_SIZE;
            use crate::types::TypeId;

            let dst = read_reg(state)?;
            let cap_reg = read_reg(state)?;

            let cap_raw = state.get_reg(cap_reg).as_i64();
            let cap: usize = if cap_raw < 16 { 16 } else { cap_raw as usize };

            let backing = state.heap.alloc(TypeId::BYTE_LIST, cap)?;
            state.record_allocation();
            // Backing data is `cap` raw bytes — no per-element
            // initialisation needed (heap.alloc returns zeroed memory
            // for managed allocations; len = 0 means no slot is read).

            let list = state
                .heap
                .alloc(TypeId::BYTE_LIST, 3 * std::mem::size_of::<Value>())?;
            state.record_allocation();
            let data_ptr = unsafe {
                (list.as_ptr() as *mut u8).add(OBJECT_HEADER_SIZE) as *mut Value
            };
            unsafe {
                *data_ptr = Value::from_i64(0);
                *data_ptr.add(1) = Value::from_i64(cap as i64);
                *data_ptr.add(2) = Value::from_ptr(backing.as_ptr() as *mut u8);
            }
            state.set_reg(dst, Value::from_ptr(list.as_ptr() as *mut u8));
            Ok(DispatchResult::Continue)
        }

        _ => Err(InterpreterError::NotImplemented {
            feature: "mem_extended sub-opcode",
            opcode: Some(Opcode::MemExtended),
        }),
    }
}

/// Format a Value for logging output.
pub(in super::super) fn format_value_for_log(value: &Value) -> String {
    if value.is_small_string() {
        // For small strings, extract the content
        value.as_small_string().as_str().to_string()
    } else if value.is_int() {
        format!("{}", value.as_i64())
    } else if value.is_float() {
        format!("{}", value.as_f64())
    } else if value.is_bool() {
        format!("{}", value.as_bool())
    } else if value.is_nil() {
        "nil".to_string()
    } else if value.is_unit() {
        "()".to_string()
    } else if value.is_ptr() {
        // Could be a char or other pointer value
        // Try to interpret as char if it looks like a valid code point
        let raw = value.as_ptr::<()>() as u64;
        if raw < 0x110000
            && let Some(c) = char::from_u32(raw as u32)
        {
            return format!("{}", c);
        }
        format!("<ptr:{:p}>", value.as_ptr::<()>())
    } else {
        format!("{:?}", value)
    }
}
