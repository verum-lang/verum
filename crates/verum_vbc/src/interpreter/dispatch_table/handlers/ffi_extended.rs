//! FFI extended opcode handler and system call helpers for VBC interpreter dispatch.
//! Note: Many variables are only used under specific `#[cfg]` feature/platform gates.
#![allow(unreachable_code, unused_variables)]

#[allow(unused_imports)]
use crate::instruction::{FfiSubOpcode, Reg};
#[allow(unused_imports)]
use crate::module::FfiSymbolId;
#[allow(unused_imports)]
use crate::types::StringId;
use crate::types::TypeId;
use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::super::heap;
use super::super::DispatchResult;
use super::bytecode_io::*;
use super::string_helpers::*;
use super::method_dispatch::{monotonic_nanos_shared, realtime_nanos_shared};

/// Maximum allowed allocation/copy size for FFI memory operations (1 GiB).
///
/// This cap prevents denial-of-service and memory-safety issues when untrusted
/// register values are passed as sizes to raw memory operations
/// (`CMemcpy`, `CMemset`, `CMemmove`, `CMemcmp`).
const MAX_FFI_ALLOCATION_SIZE: usize = 1 << 30; // 1 GiB

/// Helper: read an element from a heap-allocated array or list by index.
fn get_array_element(ptr: *const u8, header: &heap::ObjectHeader, index: usize) -> InterpreterResult<Value> {
    // SECURITY: `index * size_of::<Value>()` can overflow `usize` on huge indices,
    // producing a wrapped offset that would point into arbitrary memory. Use
    // `checked_mul` and return an overflow error if the multiplication wraps.
    let elem_offset = index
        .checked_mul(std::mem::size_of::<Value>())
        .ok_or(InterpreterError::IntegerOverflow { operation: "array_index_offset" })?;
    if header.type_id == TypeId::LIST {
        // SAFETY: `ptr` points to a heap-allocated object with a valid ObjectHeader
        // followed by Value slots. Adding OBJECT_HEADER_SIZE skips the header and
        // lands on the first Value slot (the List descriptor). Caller guarantees
        // `ptr` remains valid for the duration of this read.
        let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
        // SAFETY: List layout stores `[len, capacity, backing_ptr, ...]` inline.
        // Slot 2 holds the backing-buffer pointer — dereferencing is safe because
        // `data_ptr` is aligned and derived from a live heap allocation above.
        let backing = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
        // SAFETY: `backing` is the List's backing buffer (another heap object with
        // its own header). `elem_offset` has been bounds-checked via checked_mul
        // above, so the byte offset cannot overflow or wrap. Resulting pointer
        // stays within the allocation.
        let elem_ptr = unsafe { backing.add(heap::OBJECT_HEADER_SIZE + elem_offset) as *const Value };
        // SAFETY: `elem_ptr` is a properly aligned pointer to an initialized
        // Value slot within the backing buffer. Value is Copy, so the read does
        // not move ownership.
        Ok(unsafe { *elem_ptr })
    } else {
        // SAFETY: Non-LIST arrays store Values directly after the header.
        // `elem_offset` was checked_mul-bounded above, so the addition cannot
        // overflow, and the resulting pointer is within the allocation.
        let elem_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE + elem_offset) as *const Value };
        // SAFETY: `elem_ptr` is aligned and points to an initialized Value.
        // Value is Copy; the read is sound.
        Ok(unsafe { *elem_ptr })
    }
}


// Extended opcode handlers

/// FfiExtended (0xBC) - FFI and memory operations.
///
/// Handles FFI calling conventions, memory operations, and byte array allocation.
pub(in super::super) fn handle_ffi_extended(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let sub_op_byte = read_u8(state)?;
    let sub_op = FfiSubOpcode::from_byte(sub_op_byte);

    match sub_op {
        // ================================================================
        // Memory Operations
        // ================================================================
        Some(FfiSubOpcode::CMemcpy) => {
            // Format: dst_ptr:reg, src_ptr:reg, size:reg
            let dst_reg = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let dst_ptr = state.get_reg(dst_reg).as_ptr::<u8>();
            let src_ptr = state.get_reg(src_reg).as_ptr::<u8>();
            let size_raw = state.get_reg(size_reg).as_i64();

            // SECURITY: `size` is attacker-controlled. Reject negative values and
            // cap at MAX_FFI_ALLOCATION_SIZE to prevent oversized copies that
            // would read/write arbitrary memory or cause denial-of-service.
            if size_raw < 0 || (size_raw as u64) > MAX_FFI_ALLOCATION_SIZE as u64 {
                return Err(InterpreterError::InvalidOperand {
                    message: format!("CMemcpy: size {} exceeds maximum {} or is negative", size_raw, MAX_FFI_ALLOCATION_SIZE),
                });
            }
            let size = size_raw as usize;

            if !dst_ptr.is_null() && !src_ptr.is_null() && size > 0 {
                // SAFETY: size is bounded to <= MAX_FFI_ALLOCATION_SIZE and both
                // pointers have been null-checked. Caller is responsible for
                // ensuring regions are valid and non-overlapping per FFI contract.
                unsafe {
                    std::ptr::copy_nonoverlapping(src_ptr, dst_ptr, size);
                }
            }
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::CMemset) => {
            // Format: dst_ptr:reg, value:reg, size:reg
            let dst_reg = read_reg(state)?;
            let value_reg = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let dst_val = state.get_reg(dst_reg);
            let dst_ptr = dst_val.as_ptr::<u8>();
            let value = state.get_reg(value_reg).as_i64() as u8;
            let size_raw = state.get_reg(size_reg).as_i64();

            // SECURITY: `size` is attacker-controlled. Reject negative values and
            // cap at MAX_FFI_ALLOCATION_SIZE to prevent writing to arbitrary
            // memory regions beyond the legitimate allocation.
            if size_raw < 0 || (size_raw as u64) > MAX_FFI_ALLOCATION_SIZE as u64 {
                return Err(InterpreterError::InvalidOperand {
                    message: format!("CMemset: size {} exceeds maximum {} or is negative", size_raw, MAX_FFI_ALLOCATION_SIZE),
                });
            }
            let size = size_raw as usize;

            if !dst_ptr.is_null() && size > 0 {
                // SAFETY: size is bounded to <= MAX_FFI_ALLOCATION_SIZE and
                // dst_ptr has been null-checked. Caller is responsible for the
                // destination region being valid for size bytes.
                unsafe {
                    std::ptr::write_bytes(dst_ptr, value, size);
                }
            }
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::CMemmove) => {
            // Format: dst_ptr:reg, src_ptr:reg, size:reg
            let dst_reg = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let dst_ptr = state.get_reg(dst_reg).as_ptr::<u8>();
            let src_ptr = state.get_reg(src_reg).as_ptr::<u8>();
            let size_raw = state.get_reg(size_reg).as_i64();

            // SECURITY: `size` is attacker-controlled. Reject negative values
            // and cap at MAX_FFI_ALLOCATION_SIZE to prevent oversized copies.
            if size_raw < 0 || (size_raw as u64) > MAX_FFI_ALLOCATION_SIZE as u64 {
                return Err(InterpreterError::InvalidOperand {
                    message: format!("CMemmove: size {} exceeds maximum {} or is negative", size_raw, MAX_FFI_ALLOCATION_SIZE),
                });
            }
            let size = size_raw as usize;

            if !dst_ptr.is_null() && !src_ptr.is_null() && size > 0 {
                // SAFETY: size is bounded to <= MAX_FFI_ALLOCATION_SIZE and both
                // pointers have been null-checked. `std::ptr::copy` handles
                // overlapping regions correctly.
                unsafe {
                    std::ptr::copy(src_ptr, dst_ptr, size);
                }
            }
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::CMemcmp) => {
            // Format: dst:reg, ptr1:reg, ptr2:reg, size:reg
            let dst = read_reg(state)?;
            let ptr1_reg = read_reg(state)?;
            let ptr2_reg = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let ptr1 = state.get_reg(ptr1_reg).as_ptr::<u8>();
            let ptr2 = state.get_reg(ptr2_reg).as_ptr::<u8>();
            let size_raw = state.get_reg(size_reg).as_i64();

            // SECURITY: `size` is attacker-controlled. Reject negative values
            // and cap at MAX_FFI_ALLOCATION_SIZE to avoid constructing slices
            // from arbitrary memory (which would be a buffer over-read).
            if size_raw < 0 || (size_raw as u64) > MAX_FFI_ALLOCATION_SIZE as u64 {
                return Err(InterpreterError::InvalidOperand {
                    message: format!("CMemcmp: size {} exceeds maximum {} or is negative", size_raw, MAX_FFI_ALLOCATION_SIZE),
                });
            }
            let size = size_raw as usize;

            let result = if ptr1.is_null() || ptr2.is_null() || size == 0 {
                0i64
            } else {
                // SAFETY: size is bounded to <= MAX_FFI_ALLOCATION_SIZE and both
                // pointers have been null-checked above. Caller is responsible
                // for both regions being valid reads for `size` bytes per the
                // FFI contract of `memcmp`.
                let slice1 = unsafe { std::slice::from_raw_parts(ptr1, size) };
                let slice2 = unsafe { std::slice::from_raw_parts(ptr2, size) };
                match slice1.cmp(slice2) {
                    std::cmp::Ordering::Less => -1i64,
                    std::cmp::Ordering::Equal => 0i64,
                    std::cmp::Ordering::Greater => 1i64,
                }
            };
            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Byte Array Allocation
        // ================================================================
        Some(FfiSubOpcode::NewByteArray) => {
            // Allocate a byte array (contiguous bytes, not Values)
            // Format: dst:reg, size:reg, init:reg
            let dst = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let init_reg = read_reg(state)?;

            let size = state.get_reg(size_reg).as_i64() as usize;
            let init_byte = state.get_reg(init_reg).as_i64() as u8;

            // Allocate with TypeId::U8 to mark this as a byte array
            let obj = state.heap.alloc(TypeId::U8, size)?;
            state.record_allocation();

            // Initialize all bytes
            // SAFETY: `obj.as_ptr()` was just returned from `state.heap.alloc()`,
            // which yields a non-null pointer with `OBJECT_HEADER_SIZE + size`
            // bytes of valid storage. Skipping past the header lands on the
            // first data byte of the allocation, which stays valid until the
            // object is freed.
            let data_ptr = unsafe {
                (obj.as_ptr() as *mut u8).add(super::super::super::heap::OBJECT_HEADER_SIZE)
            };
            // SAFETY: `data_ptr` points to `size` bytes of uninitialized storage
            // inside the freshly allocated object. Writing `size` bytes stays in
            // bounds and leaves every byte initialized to `init_byte`.
            unsafe {
                std::ptr::write_bytes(data_ptr, init_byte, size);
            }

            // Return header pointer so GetE/SetE work correctly with byte arrays
            state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Byte Array Element Address
        // ================================================================
        Some(FfiSubOpcode::ByteArrayElementAddr) => {
            // Get address of element in byte array (for &mut buf[idx] as *mut Byte)
            // Format: dst:reg, arr:reg, idx:reg
            // Returns: dst = arr_ptr + OBJECT_HEADER_SIZE + idx
            let dst = read_reg(state)?;
            let arr_reg = read_reg(state)?;
            let idx_reg = read_reg(state)?;

            let arr_ptr = state.get_reg(arr_reg).as_ptr::<u8>();
            if arr_ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }

            let idx = state.get_reg(idx_reg).as_i64();

            // Verify this is a byte array
            // SAFETY: `arr_ptr` was null-checked above. Every pointer tagged via
            // `Value::from_ptr` that survives the heap's lifetime starts with an
            // `ObjectHeader`, so the cast is layout-compatible. The reference is
            // short-lived and does not outlive this dispatch call.
            let header = unsafe { &*(arr_ptr as *const super::super::super::heap::ObjectHeader) };
            if header.type_id != TypeId::U8 {
                return Err(InterpreterError::TypeMismatch {
                    expected: "byte array (TypeId::U8)",
                    got: "non-byte-array",
                    operation: "ByteArrayElementAddr",
                });
            }

            // Bounds check
            let array_size = header.size as usize;
            if idx < 0 || idx as usize >= array_size {
                return Err(InterpreterError::IndexOutOfBounds {
                    index: idx,
                    length: array_size,
                });
            }

            // Compute element address: arr_ptr + OBJECT_HEADER_SIZE + idx
            // SAFETY: `idx` was bounds-checked against `array_size`, so the
            // offset cannot exceed the allocation. The resulting pointer lies
            // within the live byte-array allocation.
            let elem_addr = unsafe {
                arr_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE + idx as usize)
            };
            state.set_reg(dst, Value::from_ptr(elem_addr));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Byte Array Load
        // ================================================================
        Some(FfiSubOpcode::ByteArrayLoad) => {
            // Load a byte from byte array
            // Format: dst:reg, arr:reg, idx:reg
            let dst = read_reg(state)?;
            let arr_reg = read_reg(state)?;
            let idx_reg = read_reg(state)?;

            let arr_ptr = state.get_reg(arr_reg).as_ptr::<u8>();
            if arr_ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }

            let idx = state.get_reg(idx_reg).as_i64();

            // Verify this is a byte array
            // SAFETY: `arr_ptr` is non-null (checked above) and points to a live
            // heap object whose layout begins with `ObjectHeader`. The borrow
            // does not escape this handler.
            let header = unsafe { &*(arr_ptr as *const super::super::super::heap::ObjectHeader) };
            if header.type_id != TypeId::U8 {
                return Err(InterpreterError::TypeMismatch {
                    expected: "byte array (TypeId::U8)",
                    got: "non-byte-array",
                    operation: "ByteArrayLoad",
                });
            }

            // Bounds check
            let array_size = header.size as usize;
            if idx < 0 || idx as usize >= array_size {
                return Err(InterpreterError::IndexOutOfBounds {
                    index: idx,
                    length: array_size,
                });
            }

            // Load byte value
            // SAFETY: `idx` was bounds-checked against `array_size`, so the
            // computed byte address lies within the live allocation. The byte
            // is always initialized (arrays are zeroed or written before load).
            let byte_val = unsafe {
                *arr_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE + idx as usize)
            };
            state.set_reg(dst, Value::from_i64(byte_val as i64));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Byte Array Store
        // ================================================================
        Some(FfiSubOpcode::ByteArrayStore) => {
            // Store a byte to byte array
            // Format: arr:reg, idx:reg, val:reg
            let arr_reg = read_reg(state)?;
            let idx_reg = read_reg(state)?;
            let val_reg = read_reg(state)?;

            let arr_ptr = state.get_reg(arr_reg).as_ptr::<u8>();
            if arr_ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }

            let idx = state.get_reg(idx_reg).as_i64();
            let val = state.get_reg(val_reg).as_i64() as u8;

            // Verify this is a byte array
            // SAFETY: `arr_ptr` is non-null (checked above) and is a live heap
            // object whose layout begins with `ObjectHeader`. The reference
            // is dropped before any mutation.
            let header = unsafe { &*(arr_ptr as *const super::super::super::heap::ObjectHeader) };
            if header.type_id != TypeId::U8 {
                return Err(InterpreterError::TypeMismatch {
                    expected: "byte array (TypeId::U8)",
                    got: "non-byte-array",
                    operation: "ByteArrayStore",
                });
            }

            // Bounds check
            let array_size = header.size as usize;
            if idx < 0 || idx as usize >= array_size {
                return Err(InterpreterError::IndexOutOfBounds {
                    index: idx,
                    length: array_size,
                });
            }

            // Store byte value
            // SAFETY: `idx` was bounds-checked against `array_size`, so the
            // computed address is within the live allocation. There are no
            // outstanding references to this byte (the `header` borrow above
            // is dropped by now).
            unsafe {
                *arr_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE + idx as usize) = val;
            }
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Typed Array Element Address
        // ================================================================
        Some(FfiSubOpcode::TypedArrayElementAddr) => {
            // Get element address for typed array with specified element size
            // Format: dst:reg, arr:reg, idx:reg, elem_size:u8
            let dst = read_reg(state)?;
            let arr_reg = read_reg(state)?;
            let idx_reg = read_reg(state)?;
            let elem_size = read_u8(state)? as usize;

            let arr_ptr = state.get_reg(arr_reg).as_ptr::<u8>();
            if arr_ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }

            let idx = state.get_reg(idx_reg).as_i64();
            if idx < 0 {
                return Err(InterpreterError::IndexOutOfBounds {
                    index: idx,
                    length: 0,
                });
            }

            // Compute element address: base + header + (idx * elem_size) with overflow checks
            let idx_usize = idx as usize;
            let offset = idx_usize.checked_mul(elem_size).ok_or({
                InterpreterError::IndexOutOfBounds { index: idx, length: 0 }
            })?;
            let total_offset = super::super::super::heap::OBJECT_HEADER_SIZE
                .checked_add(offset)
                .ok_or({
                    InterpreterError::IndexOutOfBounds { index: idx, length: 0 }
                })?;
            // SAFETY: `total_offset` was produced by checked arithmetic, so it
            // cannot wrap. `arr_ptr` is non-null and points to a live typed
            // array. The caller (Verum emitter) is responsible for ensuring
            // `idx * elem_size` stays within the allocation — this path mirrors
            // the AOT lowering, which applies the same contract.
            let elem_addr = unsafe { arr_ptr.add(total_offset) };

            state.set_reg(dst, Value::from_ptr(elem_addr));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // New Typed Array
        // ================================================================
        Some(FfiSubOpcode::NewTypedArray) => {
            // Create new typed array with specified element size
            // Format: dst:reg, count:reg, elem_size:u8, init:reg
            let dst = read_reg(state)?;
            let count_reg = read_reg(state)?;
            let elem_size = read_u8(state)? as usize;
            let init_reg = read_reg(state)?;

            let count_val = state.get_reg(count_reg).as_i64();
            if count_val < 0 {
                return Err(InterpreterError::InvalidOperand {
                    message: format!("NewTypedArray: expected non-negative count, got {}", count_val),
                });
            }
            let count = count_val as usize;
            let init_value = state.get_reg(init_reg).as_i64();

            // Total size = count * elem_size (checked to prevent overflow)
            let total_size = count.checked_mul(elem_size).ok_or({
                InterpreterError::OutOfMemory {
                    requested: usize::MAX,
                    available: 1 << 30,
                }
            })?;
            if total_size > (1 << 30) {
                return Err(InterpreterError::OutOfMemory {
                    requested: total_size,
                    available: 1 << 30,
                });
            }

            // Allocate array (using TypeId based on element size)
            let type_id = match elem_size {
                1 => TypeId::U8,
                2 => TypeId::U16,
                4 => TypeId::U32,
                8 => TypeId::U64,
                _ => TypeId::U8, // Default to byte array for unknown sizes
            };

            // Allocate using heap.alloc which returns an Object
            let obj = state.heap.alloc(type_id, total_size)?;
            state.record_allocation();

            // Get raw pointer for initialization
            let ptr = obj.as_ptr() as *mut u8;

            // Initialize elements
            // SAFETY: `ptr` was just returned from `state.heap.alloc(type_id,
            // total_size)` and has `OBJECT_HEADER_SIZE + total_size` bytes of
            // valid storage. `total_size = count * elem_size` and was checked
            // for overflow above. All subsequent writes iterate only `count`
            // elements of `elem_size` each, so they stay in bounds.
            unsafe {
                let data_ptr = ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE);
                match elem_size {
                    1 => {
                        for i in 0..count {
                            *data_ptr.add(i) = init_value as u8;
                        }
                    }
                    2 => {
                        let data_ptr = data_ptr as *mut u16;
                        for i in 0..count {
                            *data_ptr.add(i) = init_value as u16;
                        }
                    }
                    4 => {
                        let data_ptr = data_ptr as *mut u32;
                        for i in 0..count {
                            *data_ptr.add(i) = init_value as u32;
                        }
                    }
                    8 => {
                        let data_ptr = data_ptr as *mut u64;
                        for i in 0..count {
                            *data_ptr.add(i) = init_value as u64;
                        }
                    }
                    _ => {
                        // Byte-wise initialization for unknown sizes
                        for i in 0..total_size {
                            *data_ptr.add(i) = 0;
                        }
                    }
                }
            }

            state.set_reg(dst, Value::from_ptr(ptr));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Struct Field Address (#37 — atomic-stdlib runtime enabler)
        // ================================================================
        Some(FfiSubOpcode::StructFieldAddr) => {
            // Get the raw heap address of a struct field.
            //
            // Format: dst:reg, obj:reg, field_offset_lo:u8, field_offset_hi:u8
            // Returns: dst = obj_heap_ptr + OBJECT_HEADER_SIZE + field_offset
            //
            // Used by the codegen path for `&self.field as *const T`
            // — without this, the cast falls through compile_cast's
            // generic _ arm (a passthrough), and the resulting Value
            // is a register-encoded CBGR ref bit-pattern that
            // atomic_load_* misreads as a raw address (silent
            // ptr<0x1000 → returns 0).
            let dst = read_reg(state)?;
            let obj_reg = read_reg(state)?;
            let offset_lo = read_u8(state)? as u16;
            let offset_hi = read_u8(state)? as u16;
            let field_offset = (offset_hi << 8) | offset_lo;

            let obj_val = state.get_reg(obj_reg);
            // The struct receiver lives in the register either as an
            // Object/Pointer Value (heap-allocated struct) or — for
            // some single-field receivers — as the inline payload
            // itself.  Treat both: if it's a pointer Value, take the
            // raw address; otherwise read as i64 (already the
            // address, e.g. when the field-of-self pattern was
            // pre-stabilised through an integer slot).
            let obj_ptr: *mut u8 = if obj_val.is_ptr() {
                obj_val.as_ptr()
            } else {
                obj_val.as_i64() as *mut u8
            };
            if obj_ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }

            // SAFETY: the codegen only emits StructFieldAddr when
            // (a) the receiver Type is a registered struct in the
            // type-field-layouts table, and (b) the field_offset
            // came from compute_field_offset for that type — so it
            // is always within the object's data section.
            // OBJECT_HEADER_SIZE adds the standard 24-byte header
            // skip to reach the data section's first byte.
            let field_addr = unsafe {
                obj_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE + field_offset as usize)
            };
            state.set_reg(dst, Value::from_ptr(field_addr));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Raw Pointer Operations
        // ================================================================
        Some(FfiSubOpcode::DerefRaw) => {
            // Read value through raw pointer
            // Format: dst:reg, ptr:reg, size:u8
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let size = read_u8(state)?;

            // Accept both Pointer-tagged and Int-tagged values as raw addresses
            let val = state.get_reg(ptr_reg);
            let ptr: *mut u8 = if val.is_ptr() {
                val.as_ptr()
            } else {
                val.as_i64() as *mut u8
            };
            if ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }

            // SAFETY: `ptr` was null-checked above. The caller of the
            // `DerefRaw` opcode is responsible for supplying a pointer that is
            // valid for reads of `size` bytes — this mirrors Rust's raw-pointer
            // semantics and matches the AOT lowering. `read_unaligned` handles
            // arbitrary alignment.
            //
            // Extension policy (root fix for Issue #2 — continuation from
            // `handle_get_index`): `DerefRaw` is emitted by typed-array
            // reads and by any intrinsic that lowers to a raw memory read.
            // Widths 1/2/4 are *zero-extended* into the i64 NaN-box slot so
            // that a `[UInt32; N]` (or `[UInt8; N]` / `[UInt16; N]`) element
            // whose high bit is set is preserved as an unsigned value.
            // Sign-extension via `as i32 as i64` corrupted the upper 32 bits
            // of every u32 with bit 31 set and was the root cause of the
            // CRC32 divergence against zlib. Callers that actually need
            // signed semantics for a raw read can truncate (`as i32` etc.)
            // at the use site — zero-extension is the invariant-preserving
            // default for unsigned raw I/O, which is the vastly common case.
            let value = unsafe {
                match size {
                    1 => *ptr as i64,                                                 // u8 → i64 (zero-extend)
                    2 => std::ptr::read_unaligned(ptr as *const u16) as i64,          // u16 → i64
                    4 => std::ptr::read_unaligned(ptr as *const u32) as i64,          // u32 → i64
                    8 => std::ptr::read_unaligned(ptr as *const i64),                 // 8 bytes fill the slot
                    _ => return Err(InterpreterError::InvalidOperand {
                        message: format!("invalid deref size: {}", size),
                    }),
                }
            };
            state.set_reg(dst, Value::from_i64(value));
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::DerefMutRaw) => {
            // Write value through raw pointer
            // Format: ptr:reg, value:reg, size:u8
            let ptr_reg = read_reg(state)?;
            let value_reg = read_reg(state)?;
            let size = read_u8(state)?;

            // SECURITY: Only accept Pointer-tagged values for write operations.
            // Casting arbitrary integers to pointers enables arbitrary memory writes.
            let val = state.get_reg(ptr_reg);
            let ptr: *mut u8 = if val.is_ptr() {
                val.as_ptr()
            } else {
                return Err(InterpreterError::InvalidOperand {
                    message: format!("DerefMutRaw requires pointer-tagged value, got integer: {}", val.as_i64()),
                });
            };
            if ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }

            let value = state.get_reg(value_reg).as_i64();
            // SAFETY: `ptr` was null-checked above AND rejected if it was not a
            // pointer-tagged Value (guards against arbitrary integer-to-pointer
            // writes). The caller is responsible for ensuring the target is
            // writable for `size` bytes per the FFI contract. `write_unaligned`
            // handles arbitrary alignment.
            unsafe {
                match size {
                    1 => *ptr = value as u8,
                    2 => std::ptr::write_unaligned(ptr as *mut i16, value as i16),
                    4 => std::ptr::write_unaligned(ptr as *mut i32, value as i32),
                    8 => std::ptr::write_unaligned(ptr as *mut i64, value),
                    _ => return Err(InterpreterError::InvalidOperand {
                        message: format!("invalid deref size: {}", size),
                    }),
                }
            }
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::DerefRawPtr) => {
            // Read pointer through raw pointer (for pointer-to-pointer)
            // Format: dst:reg, ptr:reg
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;

            let ptr = state.get_reg(ptr_reg).as_ptr::<*mut u8>();
            if ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }

            // SAFETY: `ptr` is non-null (checked above) and is a pointer-to-pointer
            // produced by the caller. Aligned reads of a `*mut u8` are sound so
            // long as the target holds a valid pointer bit-pattern, per the FFI
            // contract for `DerefRawPtr`.
            let value_ptr = unsafe { *ptr };
            state.set_reg(dst, Value::from_ptr(value_ptr));
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::PtrAdd) => {
            // Pointer arithmetic: add offset
            // Format: dst:reg, ptr:reg, offset:reg
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let offset_reg = read_reg(state)?;

            let ptr = state.get_reg(ptr_reg).as_ptr::<u8>();
            let offset = state.get_reg(offset_reg).as_i64();

            // SECURITY: `ptr.add(offset)` uses wrapping arithmetic on the raw
            // address, which can wrap around the address space when `offset`
            // is attacker-controlled, producing an arbitrary pointer. Use
            // `checked_add_signed`/`checked_sub` on the address bits and
            // return an error on overflow.
            let addr = ptr as usize;
            let new_addr = if offset >= 0 {
                addr.checked_add(offset as usize)
            } else {
                // offset is negative; subtract its absolute value from addr
                let abs = (offset as i128).unsigned_abs() as usize;
                addr.checked_sub(abs)
            };
            let new_addr = new_addr.ok_or(InterpreterError::IntegerOverflow {
                operation: "PtrAdd",
            })?;
            let result = new_addr as *mut u8;
            state.set_reg(dst, Value::from_ptr(result));
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::PtrSub) => {
            // Pointer arithmetic: subtract offset
            // Format: dst:reg, ptr:reg, offset:reg
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let offset_reg = read_reg(state)?;

            let ptr = state.get_reg(ptr_reg).as_ptr::<u8>();
            let offset = state.get_reg(offset_reg).as_i64();

            // SECURITY: raw pointer `.sub`/`.add` wrap around the address
            // space with an attacker-controlled offset. Perform checked
            // arithmetic on the integer address and fail on overflow.
            let addr = ptr as usize;
            let new_addr = if offset >= 0 {
                addr.checked_sub(offset as usize)
            } else {
                let abs = (offset as i128).unsigned_abs() as usize;
                addr.checked_add(abs)
            };
            let new_addr = new_addr.ok_or(InterpreterError::IntegerOverflow {
                operation: "PtrSub",
            })?;
            let result = new_addr as *mut u8;
            state.set_reg(dst, Value::from_ptr(result));
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::PtrDiff) => {
            // Pointer difference: compute distance in bytes
            // Format: dst:reg, ptr1:reg, ptr2:reg
            let dst = read_reg(state)?;
            let ptr1_reg = read_reg(state)?;
            let ptr2_reg = read_reg(state)?;

            let ptr1 = state.get_reg(ptr1_reg).as_ptr::<u8>();
            let ptr2 = state.get_reg(ptr2_reg).as_ptr::<u8>();

            let diff = (ptr1 as isize) - (ptr2 as isize);
            state.set_reg(dst, Value::from_i64(diff as i64));
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::PtrIsNull) => {
            // Check if pointer is null
            // Format: dst:reg, ptr:reg
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;

            let ptr = state.get_reg(ptr_reg).as_ptr::<u8>();
            let is_null = ptr.is_null();
            state.set_reg(dst, Value::from_bool(is_null));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // C-style memory allocation
        // ================================================================
        Some(FfiSubOpcode::CAlloc) => {
            // Format: dst:reg, size:reg
            let dst = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let size = state.get_reg(size_reg).as_i64() as usize;

            if size == 0 {
                state.set_reg(dst, Value::from_ptr(std::ptr::null_mut::<u8>()));
            } else {
                let layout = std::alloc::Layout::from_size_align(size, 8)
                    .map_err(|_| InterpreterError::InvalidOperand {
                        message: format!("invalid allocation size: {}", size),
                    })?;
                // SAFETY: `layout` has a non-zero `size` (the `size == 0` path
                // is handled above) and a valid alignment, so `alloc_zeroed`
                // satisfies its precondition. A null return is tolerated by
                // `Value::from_ptr` and surfaced to Verum code.
                let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
                state.set_reg(dst, Value::from_ptr(ptr));
            }
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::CFree) => {
            // Format: ptr:reg, size:reg
            let ptr_reg = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let ptr = state.get_reg(ptr_reg).as_ptr::<u8>();
            let size = state.get_reg(size_reg).as_i64() as usize;

            if !ptr.is_null() && size > 0 {
                let layout = std::alloc::Layout::from_size_align(size, 8)
                    .map_err(|_| InterpreterError::InvalidOperand {
                        message: format!("invalid deallocation size: {}", size),
                    })?;
                // SAFETY: Caller warrants that `ptr` was returned from a
                // previous `CAlloc` with the same `size`/alignment, and is not
                // deallocated twice. We null-checked `ptr` and rejected
                // `size == 0`, so the `Layout` precondition is satisfied.
                unsafe { std::alloc::dealloc(ptr, layout) };
            }
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Random number generation
        // ================================================================
        Some(FfiSubOpcode::RandomU64) => {
            // Format: dst:reg
            let dst = read_reg(state)?;

            let random_value: u64 = {
                #[cfg(target_os = "macos")]
                {
                    let mut buf = [0u8; 8];
                    // SAFETY: `buf` is a stack-local 8-byte array; passing its
                    // mutable pointer and length 8 to `getentropy` satisfies
                    // the POSIX contract (max 256 bytes, pointer writable for
                    // length bytes).
                    unsafe {
                        libc::getentropy(buf.as_mut_ptr() as *mut libc::c_void, 8);
                    }
                    u64::from_ne_bytes(buf)
                }

                #[cfg(target_os = "linux")]
                {
                    let mut buf = [0u8; 8];
                    // SAFETY: `buf` is a stack-local 8-byte array. The getrandom
                    // syscall writes at most `len` bytes to `buf`. Using the
                    // raw syscall number for the current architecture is
                    // intentional: some libc builds lack `SYS_getrandom`.
                    unsafe {
                        #[cfg(target_arch = "x86_64")]
                        let syscall_num = 318i64;
                        #[cfg(target_arch = "aarch64")]
                        let syscall_num = 278i64;
                        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
                        let syscall_num = libc::SYS_getrandom;
                        libc::syscall(syscall_num, buf.as_mut_ptr(), 8usize, 0u32);
                    }
                    u64::from_ne_bytes(buf)
                }

                #[cfg(not(any(target_os = "macos", target_os = "linux")))]
                {
                    let timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(0);
                    let mut x = timestamp ^ 0x5DEECE66D;
                    x ^= x >> 12;
                    x ^= x << 25;
                    x ^= x >> 27;
                    x.wrapping_mul(0x2545F4914F6CDD1D)
                }
            };

            state.set_reg(dst, Value::from_i64(random_value as i64));
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::RandomFloat) => {
            // Format: dst:reg
            let dst = read_reg(state)?;

            let random_u64: u64 = {
                #[cfg(target_os = "macos")]
                {
                    let mut buf = [0u8; 8];
                    // SAFETY: `buf` is a stack-local 8-byte array. See RandomU64
                    // macos branch — same reasoning applies.
                    unsafe {
                        libc::getentropy(buf.as_mut_ptr() as *mut libc::c_void, 8);
                    }
                    u64::from_ne_bytes(buf)
                }

                #[cfg(target_os = "linux")]
                {
                    let mut buf = [0u8; 8];
                    // SAFETY: Same as RandomU64 linux branch — `buf` is a
                    // stack-local 8-byte array and the kernel writes at most
                    // `len` bytes.
                    unsafe {
                        #[cfg(target_arch = "x86_64")]
                        let syscall_num = 318i64;
                        #[cfg(target_arch = "aarch64")]
                        let syscall_num = 278i64;
                        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
                        let syscall_num = libc::SYS_getrandom;
                        libc::syscall(syscall_num, buf.as_mut_ptr(), 8usize, 0u32);
                    }
                    u64::from_ne_bytes(buf)
                }

                #[cfg(not(any(target_os = "macos", target_os = "linux")))]
                {
                    let timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(0);
                    let mut x = timestamp ^ 0x5DEECE66D;
                    x ^= x >> 12;
                    x ^= x << 25;
                    x ^= x >> 27;
                    x.wrapping_mul(0x2545F4914F6CDD1D)
                }
            };

            // IEEE 754 conversion: (bits >> 11) * (1.0 / 2^53)
            let float_value = (random_u64 >> 11) as f64 * (1.0 / ((1u64 << 53) as f64));
            state.set_reg(dst, Value::from_f64(float_value));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Errno operations
        // ================================================================
        Some(FfiSubOpcode::GetErrno) => {
            let dst = read_reg(state)?;
            #[cfg(feature = "ffi")]
            {
                let ffi_runtime = state.get_or_create_ffi_runtime()?;
                let errno = ffi_runtime.get_errno();
                state.set_reg(dst, Value::from_i64(errno as i64));
            }
            #[cfg(not(feature = "ffi"))]
            {
                state.set_reg(dst, Value::from_i64(0));
            }
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::SetErrno) => {
            let src = read_reg(state)?;
            let val = state.get_reg(src);
            #[cfg(feature = "ffi")]
            {
                let ffi_runtime = state.get_or_create_ffi_runtime()?;
                if val.is_int() {
                    ffi_runtime.set_errno(val.as_i64() as i32);
                }
            }
            #[cfg(not(feature = "ffi"))]
            {
                let _ = val;
            }
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::ClearErrno) => {
            #[cfg(feature = "ffi")]
            {
                let ffi_runtime = state.get_or_create_ffi_runtime()?;
                ffi_runtime.clear_errno();
            }
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::GetLastError) => {
            let dst = read_reg(state)?;
            // Not implemented on Unix
            state.set_reg(dst, Value::from_i64(0));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // FFI Array Marshalling: Verum array → C contiguous buffer
        // ================================================================
        Some(FfiSubOpcode::ArrayToC) => {
            // Format: dst:reg, arr_reg:reg, idx_reg:reg, element_type:u8, is_mutable:u8
            let dst_reg = read_reg(state)?;
            let arr_reg = read_reg(state)?;
            let _idx_reg = read_reg(state)?; // Currently only index 0 is supported
            let element_type = read_u8(state)?;
            let is_mutable = read_u8(state)? != 0;

            #[cfg(feature = "ffi")]
            {
                let arr_val = state.get_reg(arr_reg);
                if !arr_val.is_ptr() {
                    return Err(InterpreterError::TypeMismatch {
                        expected: "array pointer",
                        got: "non-pointer",
                        operation: "ArrayToC",
                    });
                }

                let arr_ptr = arr_val.as_ptr::<u8>();
                // SAFETY: `arr_val.is_ptr()` was just checked, so `arr_ptr` is
                // non-null and tagged as a pointer. All pointer-tagged values
                // in VBC originate from `state.heap.alloc()`, whose objects
                // begin with `ObjectHeader`. The borrow is read-only and does
                // not outlive the enclosing block.
                let header = unsafe { &*(arr_ptr as *const super::super::super::heap::ObjectHeader) };
                let header_size = header.size as usize;

                // Determine element size in bytes
                let elem_size: usize = match element_type {
                    0x01 => 1,  // i8/u8
                    0x02 => 2,  // i16/u16
                    0x03 => 4,  // i32/u32
                    0x04 => 8,  // i64/u64
                    0x05 => 4,  // f32
                    0x06 => 8,  // f64
                    _ => 8,     // default i64
                };

                // Calculate element count based on array type
                let is_typed_array = matches!(header.type_id, t if t == TypeId::U16 || t == TypeId::U32 || t == TypeId::U64);
                let count = if header.type_id == TypeId::LIST {
                    // SAFETY: List layout stores `[len, capacity, backing, ...]`
                    // inline after the header. `arr_ptr` points to a live List
                    // allocation (verified via `type_id == LIST`), so adding
                    // OBJECT_HEADER_SIZE lands on the `len` slot.
                    let data_ptr = unsafe { arr_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE) as *const Value };
                    // SAFETY: `data_ptr` is aligned and points to the
                    // initialized `len` slot of the List header.
                    (unsafe { (*data_ptr).as_i64() }) as usize
                } else if header.type_id == TypeId::U8 {
                    header_size
                } else if is_typed_array {
                    // Typed array: element count = header_size / native element size
                    let native_elem_size = match header.type_id {
                        t if t == TypeId::U16 => 2,
                        t if t == TypeId::U32 => 4,
                        _ => 8, // U64
                    };
                    header_size / native_elem_size
                } else {
                    header_size / std::mem::size_of::<Value>()
                };

                // Allocate C buffer (checked multiplication to prevent overflow)
                let buf_size = count.checked_mul(elem_size).ok_or({
                    InterpreterError::AllocationTooLarge {
                        requested: usize::MAX,
                        max_allowed: 1 << 30,
                    }
                })?;
                if buf_size > (1 << 30) {
                    return Err(InterpreterError::OutOfMemory {
                        requested: buf_size,
                        available: 1 << 30,
                    });
                }
                let layout = std::alloc::Layout::from_size_align(buf_size.max(8), elem_size.max(8))
                    .unwrap_or(std::alloc::Layout::new::<u64>());
                // SAFETY: `layout` has a non-zero size (at least 8) and a valid
                // power-of-two alignment, satisfying `alloc`'s precondition.
                // A null return would leave `buffer` dangling, but that is
                // handled later by the FFI cleanup path.
                let buffer = unsafe { std::alloc::alloc(layout) };

                if is_typed_array {
                    // For typed arrays, copy raw bytes directly (data is already in native format)
                    // SAFETY: `arr_ptr` is a live heap object with at least
                    // `OBJECT_HEADER_SIZE + buf_size` bytes (typed arrays pack
                    // their native elements after the header).
                    let src = unsafe { arr_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE) };
                    // SAFETY: `buffer` was just allocated with capacity
                    // `buf_size.max(8)`; `src` has at least `buf_size` bytes
                    // of live storage; the two regions are non-overlapping
                    // since `buffer` is a fresh allocation.
                    unsafe { std::ptr::copy_nonoverlapping(src, buffer, buf_size) };
                } else {
                    // Marshal each element from NaN-boxed Value to C type
                    for i in 0..count {
                        let elem = get_array_element(arr_ptr, header, i)?;
                        // SAFETY: `buffer` has `buf_size = count * elem_size`
                        // bytes; each write below indexes `i < count` elements
                        // of `elem_size` bytes, staying in bounds. The element
                        // type sizes are fixed (1/2/4/8) so alignment is
                        // respected via the raw-byte `add(i * elem_size)`
                        // offset and the match-arm cast.
                        unsafe {
                            match element_type {
                                0x01 => { // i8
                                    *(buffer.add(i) as *mut i8) = elem.as_i64() as i8;
                                }
                                0x02 => { // i16
                                    *(buffer.add(i * 2) as *mut i16) = elem.as_i64() as i16;
                                }
                                0x03 => { // i32
                                    *(buffer.add(i * 4) as *mut i32) = elem.as_i64() as i32;
                                }
                                0x04 => { // i64
                                    *(buffer.add(i * 8) as *mut i64) = elem.as_i64();
                                }
                                0x05 => { // f32
                                    *(buffer.add(i * 4) as *mut f32) = elem.as_f64() as f32;
                                }
                                0x06 => { // f64
                                    *(buffer.add(i * 8) as *mut f64) = elem.as_f64();
                                }
                                _ => {
                                    *(buffer.add(i * 8) as *mut i64) = elem.as_i64();
                                }
                            }
                        }
                    }
                }

                // Store buffer for cleanup/writeback
                state.ffi_array_buffers.push(super::super::super::state::FfiArrayBuffer {
                    buffer,
                    layout,
                    array_obj_ptr: arr_ptr,
                    count,
                    element_type,
                    is_mutable,
                });

                // Set dst register to the buffer pointer
                state.set_reg(dst_reg, Value::from_ptr(buffer));
            }
            #[cfg(not(feature = "ffi"))]
            {
                let _ = (dst_reg, arr_reg, element_type, is_mutable);
                return Err(InterpreterError::NotImplemented {
                    feature: "FFI array marshalling requires the 'ffi' feature",
                    opcode: None,
                });
            }

            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // FFI Create Callback Trampoline (Verum function → C function pointer)
        // ================================================================
        Some(FfiSubOpcode::CreateCallback) => {
            // Format: dst:reg, fn_id:u32, signature_idx:u32
            let dst_reg = read_reg(state)?;
            let fn_id = read_u32(state)?;
            let signature_idx = read_u32(state)?;

            #[cfg(feature = "ffi")]
            {
                let module = state.module.clone();
                let ffi_runtime = state.get_or_create_ffi_runtime()?;

                // Create the trampoline using the existing infrastructure
                let code_ptr = ffi_runtime.create_callback_from_symbol(&module, fn_id, signature_idx)
                    .map_err(|e| InterpreterError::FfiRuntimeError(format!("{}", e)))?;

                state.set_reg(dst_reg, Value::from_ptr(code_ptr as *mut u8));
            }
            #[cfg(not(feature = "ffi"))]
            {
                let _ = (dst_reg, fn_id, signature_idx);
                return Err(InterpreterError::NotImplemented {
                    feature: "FFI callbacks require the 'ffi' feature",
                    opcode: None,
                });
            }

            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // FFI Call (C calling convention)
        // ================================================================
        Some(FfiSubOpcode::CallFfiC) => {
            // Format: symbol_idx:u32, arg_count:u8, ret_reg:reg, [arg_regs...],
            //         mut_ref_count:u8, [(arg_idx:u8, source_reg:reg)...]
            let symbol_idx = read_u32(state)?;
            let arg_count = read_u8(state)? as usize;
            let ret_reg = read_reg(state)?;

            // Read argument registers
            let mut arg_regs = Vec::with_capacity(arg_count);
            for _ in 0..arg_count {
                arg_regs.push(read_reg(state)?);
            }

            // Read mutable reference source register map
            let mut_ref_count = read_u8(state)? as usize;
            let mut source_reg_map = std::collections::HashMap::new();
            for _ in 0..mut_ref_count {
                let arg_idx = read_u8(state)?;
                let source_reg = read_reg(state)?;
                source_reg_map.insert(arg_idx, source_reg.0);
            }

            // Collect argument values
            let args: Vec<Value> = arg_regs.iter().map(|r| state.get_reg(*r)).collect();

            #[cfg(feature = "ffi")]
            {
                let symbol_id = FfiSymbolId(symbol_idx);

                // Load module libraries if not yet loaded
                let module = state.module.clone();

                // Set up callback handler for re-entrant calls from C
                // Must be done before getting ffi_runtime to avoid double borrow
                state.setup_callback_handler();

                let ffi_runtime = state.get_or_create_ffi_runtime()?;
                // Load libraries (idempotent)
                ffi_runtime.load_module_libraries(&module)
                    .map_err(|e| InterpreterError::FfiRuntimeError(format!("{}", e)))?;

                // Pre-call: clear errno for error-protocol functions
                {
                    if let Some(sym) = module.get_ffi_symbol(symbol_id)
                        && matches!(sym.error_protocol,
                            crate::module::ErrorProtocol::NegOneErrno
                            | crate::module::ErrorProtocol::NullErrno
                            | crate::module::ErrorProtocol::ReturnCodePattern
                            | crate::module::ErrorProtocol::SentinelWithErrno
                        ) {
                            ffi_runtime.clear_errno();
                        }
                }

                // Call the FFI function
                let mut ret_value = Value::nil();
                // SAFETY: `call_module_ffi_c_with_writeback_v2` invokes a
                // native symbol resolved from `module`'s library list; the
                // caller trusts that the declared C signature matches the
                // underlying symbol. `args` and `source_reg_map` reference
                // live register slots for the duration of the call.
                let writebacks = unsafe {
                    ffi_runtime.call_module_ffi_c_with_writeback_v2(
                        &module,
                        symbol_id,
                        &args,
                        &source_reg_map,
                        &mut ret_value,
                    )
                }.map_err(|e| InterpreterError::FfiRuntimeError(format!("{}", e)))?;

                // If return type is a struct, convert raw C buffer to Verum heap object
                if let Some(sym) = module.get_ffi_symbol(symbol_id)
                    && matches!(sym.signature.return_type, crate::module::CType::StructValue)
                        && let Some(layout_idx) = sym.signature.return_layout_idx {
                            let layout = &module.ffi_layouts[layout_idx as usize];
                            let c_buf_ptr = ret_value.as_ptr::<u8>();
                            if !c_buf_ptr.is_null() {
                                // Determine number of Value slots needed
                                let max_field_idx = layout.fields.iter()
                                    .map(|f| f.name.0 as usize)
                                    .max()
                                    .unwrap_or(0);
                                let slot_count = max_field_idx + 1;
                                let type_id = layout.verum_type.unwrap_or(crate::types::TypeId::UNIT);
                                let obj = state.heap.alloc(type_id, slot_count * std::mem::size_of::<Value>())?;
                                state.record_allocation();

                                // SAFETY: `obj` was just returned from
                                // `state.heap.alloc()` with exactly
                                // `slot_count * sizeof(Value)` data bytes, so
                                // the Value slice runs from `data_ptr` to
                                // `data_ptr.add(slot_count)`.
                                let data_ptr = unsafe {
                                    (obj.as_ptr() as *mut u8).add(super::super::super::heap::OBJECT_HEADER_SIZE) as *mut Value
                                };
                                // Marshal each field from C buffer into heap object
                                for field in &layout.fields {
                                    // SAFETY: `field.offset` is provided by the
                                    // compiler-emitted layout table; the C
                                    // buffer is exactly the 256-byte box below
                                    // and was just written to by the FFI call.
                                    // The field offsets are bounded by that
                                    // box size per the layout contract.
                                    let c_field_ptr = unsafe { c_buf_ptr.add(field.offset as usize) };
                                    // SAFETY: `marshal_field_from_c` reads a
                                    // typed field from `c_field_ptr` whose
                                    // layout matches `field.c_type`. The
                                    // layout table is generated alongside the
                                    // FFI signature so the pairing is
                                    // guaranteed.
                                    if let Some(val) = unsafe { crate::ffi::runtime::marshal_field_from_c(field.c_type, c_field_ptr) } {
                                        let slot_idx = field.name.0 as usize;
                                        // SAFETY: `slot_idx` is bounded by
                                        // `max_field_idx`, and `data_ptr` has
                                        // `slot_count = max_field_idx + 1`
                                        // Value slots of writable storage.
                                        unsafe { *data_ptr.add(slot_idx) = val; }
                                    }
                                }
                                // Free the C buffer (was Box::into_raw'd [0u8; 256])
                                // SAFETY: The FFI runtime produced `c_buf_ptr`
                                // via `Box::into_raw` of a boxed `[0u8; 256]`.
                                // Reconstructing the same-sized slice and
                                // dropping it here reverses that allocation
                                // exactly once per non-null return.
                                unsafe { let _ = Box::from_raw(std::ptr::slice_from_raw_parts_mut(c_buf_ptr, 256)); }
                                ret_value = Value::from_ptr(obj.as_ptr() as *mut u8);
                            }
                        }

                // Error protocol checking: inspect return value + errno
                // Mirrors LLVM lowering logic for differential correctness.
                if let Some(sym) = module.get_ffi_symbol(symbol_id) {
                    ret_value = apply_error_protocol(state, sym, ret_value)?;
                }

                // Write return value
                state.set_reg(ret_reg, ret_value);

                // Apply write-backs for mutable reference arguments
                for (reg_idx, new_val) in writebacks {
                    state.set_reg(Reg(reg_idx), new_val);
                }

                // Apply array write-backs: copy C buffer data back to Verum arrays
                for buf in state.ffi_array_buffers.drain(..) {
                    if buf.is_mutable && !buf.buffer.is_null() && !buf.array_obj_ptr.is_null() {
                        // SAFETY: `array_obj_ptr` was stored in this buffer
                        // from the prior `ArrayToC` marshalling step, where
                        // we verified it was a live heap object (headered).
                        // Null/mutability were just checked. The borrow does
                        // not escape this scope.
                        let header = unsafe { &*(buf.array_obj_ptr as *const super::super::super::heap::ObjectHeader) };

                        // For typed arrays (U16, U32, U64), copy raw bytes directly
                        // instead of converting to Values (which would corrupt the layout)
                        if header.type_id == TypeId::U16 || header.type_id == TypeId::U32 || header.type_id == TypeId::U64 {
                            let elem_size = match header.type_id {
                                t if t == TypeId::U16 => 2usize,
                                t if t == TypeId::U32 => 4usize,
                                _ => 8usize, // U64
                            };
                            let total_bytes = buf.count.checked_mul(elem_size).unwrap_or(0);
                            // SAFETY: `array_obj_ptr` has at least
                            // `OBJECT_HEADER_SIZE + buf.count * elem_size`
                            // bytes (the original allocation stored the typed
                            // elements packed after the header).
                            let dst = unsafe { buf.array_obj_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE) };
                            // SAFETY: `buf.buffer` holds at least
                            // `total_bytes` live bytes (it was sized via the
                            // same `count * elem_size` computation on entry).
                            // `dst` has matching capacity; the regions do not
                            // overlap since `buf.buffer` is a separate alloc.
                            unsafe {
                                std::ptr::copy_nonoverlapping(buf.buffer, dst, total_bytes);
                            }
                        } else {
                            for i in 0..buf.count {
                                // SAFETY: `buf.buffer` was sized to hold
                                // `buf.count` elements of the declared
                                // element type. Each branch accesses byte
                                // offset `i * elem_size` which is within
                                // bounds.
                                let new_val = unsafe {
                                    match buf.element_type {
                                        0x01 => Value::from_i64(*(buf.buffer.add(i) as *const i8) as i64),
                                        0x02 => Value::from_i64(*(buf.buffer.add(i * 2) as *const i16) as i64),
                                        0x03 => Value::from_i64(*(buf.buffer.add(i * 4) as *const i32) as i64),
                                        0x04 => Value::from_i64(*(buf.buffer.add(i * 8) as *const i64)),
                                        0x05 => Value::from_f64(*(buf.buffer.add(i * 4) as *const f32) as f64),
                                        0x06 => Value::from_f64(*(buf.buffer.add(i * 8) as *const f64)),
                                        _ => Value::from_i64(*(buf.buffer.add(i * 8) as *const i64)),
                                    }
                                };
                                // Write element back into array
                                let elem_offset = i * std::mem::size_of::<Value>();
                                let elem_ptr = if header.type_id == TypeId::LIST {
                                    // SAFETY: List layout — see get_array_element
                                    // comments. `array_obj_ptr` is live and
                                    // starts with ObjectHeader + List header.
                                    let data_ptr = unsafe { buf.array_obj_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE) as *const Value };
                                    // SAFETY: Slot 2 of the List header holds
                                    // the backing-buffer pointer.
                                    let backing = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
                                    // SAFETY: `elem_offset = i * sizeof(Value)`
                                    // with `i < buf.count <= List length`, so
                                    // the computed address stays within the
                                    // backing buffer.
                                    unsafe { backing.add(super::super::super::heap::OBJECT_HEADER_SIZE + elem_offset) as *mut Value }
                                } else {
                                    // SAFETY: Non-LIST arrays store Values
                                    // packed after the header; `i < buf.count`
                                    // so the offset is in bounds.
                                    unsafe { buf.array_obj_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE + elem_offset) as *mut Value }
                                };
                                // SAFETY: `elem_ptr` is aligned (Values have
                                // 8-byte alignment, header + multiples of
                                // sizeof(Value) preserve that) and points to
                                // a writable Value slot.
                                unsafe { *elem_ptr = new_val; }
                            }
                        }
                    }
                    // buf is dropped here, deallocating the C buffer
                }

                // Tear down callback handler
                state.teardown_callback_handler();
            }
            #[cfg(not(feature = "ffi"))]
            {
                // FFI feature is not compiled in, but a handful of memory
                // syscalls are load-bearing for the stdlib allocator
                // bootstrap (Shared/Heap/List/Map all end here via
                // core/mem/allocator.vr:os_mmap). Route the whitelisted
                // set to Rust's std allocator so interpreter-mode code
                // can allocate; unknown symbols still surface
                // NotImplemented so real FFI work is visible.
                let _ = source_reg_map;
                let symbol_name = state.module.get_ffi_symbol(FfiSymbolId(symbol_idx))
                    .and_then(|s| state.module.strings.get(s.name))
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                let result = match symbol_name.as_str() {
                    "mmap" => {
                        let len = args.get(1).copied().map(|v| v.as_integer_compatible() as usize).unwrap_or(0);
                        let fd = args.get(4).copied().map(|v| v.as_integer_compatible()).unwrap_or(-1);
                        if fd != -1 || len == 0 {
                            Value::from_i64(-1)
                        } else {
                            let layout = std::alloc::Layout::from_size_align(len.max(8), 16)
                                .unwrap_or(std::alloc::Layout::new::<u64>());
                            let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
                            if ptr.is_null() {
                                Value::from_i64(-1)
                            } else {
                                state.cbgr_allocations.insert(ptr as usize);
                                Value::from_i64(ptr as i64)
                            }
                        }
                    }
                    "munmap" | "mprotect" | "madvise" | "msync" | "mlock" | "munlock" => {
                        Value::from_i64(0)
                    }
                    "vm_allocate" | "mach_vm_allocate" => Value::from_i64(0),
                    "pthread_self" | "mach_thread_self" | "gettid" => Value::from_i64(1),
                    _ => {
                        let _ = symbol_idx;
                        return Err(InterpreterError::NotImplemented {
                            feature: "FFI calls require the 'ffi' feature (unhandled symbol)",
                            opcode: None,
                        });
                    }
                };
                state.set_reg(ret_reg, result);
            }

            Ok(DispatchResult::Continue)
        }

        // ==============================================================
        // Time Operations (0x70-0x75)
        // ==============================================================

        Some(FfiSubOpcode::TimeMonotonicNanos) => {
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(monotonic_nanos_shared()));
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::TimeRealtimeNanos) => {
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(realtime_nanos_shared()));
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::TimeMonotonicRawNanos) => {
            // Same as MonotonicNanos for VBC interpreter (no NTP distinction)
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(monotonic_nanos_shared()));
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::TimeSleepNanos) => {
            let nanos_reg = read_reg(state)?;
            let nanos = state.get_reg(nanos_reg).as_i64();
            if nanos > 0 {
                std::thread::sleep(std::time::Duration::from_nanos(nanos as u64));
            }
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::TimeThreadCpuNanos) => {
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(monotonic_nanos_shared()));
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::TimeProcessCpuNanos) => {
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(monotonic_nanos_shared()));
            Ok(DispatchResult::Continue)
        }

        // ==============================================================
        // System Call Operations (0x80-0x85)
        // ==============================================================

        Some(FfiSubOpcode::SysGetpid) => {
            let dst = read_reg(state)?;
            let pid = std::process::id();
            state.set_reg(dst, Value::from_i64(pid as i64));
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::SysGettid) => {
            let dst = read_reg(state)?;
            #[cfg(unix)]
            let tid: u64 = {
                let mut tid: u64 = 0;
                // SAFETY: `tid` is a live stack u64. `pthread_threadid_np` writes
                // exactly one u64 via the provided pointer when the first arg is
                // 0 (self). The Apple libc contract is well-defined.
                #[cfg(target_os = "macos")]
                unsafe { libc::pthread_threadid_np(0, &mut tid); }
                #[cfg(not(target_os = "macos"))]
                {
                    // On other Unix, use the thread id as a hash of the thread handle
                    let id = std::thread::current().id();
                    tid = format!("{:?}", id).chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0);
                }
                tid
            };
            #[cfg(windows)]
            let tid: u64 = {
                // SAFETY: GetCurrentThreadId is always safe and takes no pointer arguments.
                unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() as u64 }
            };
            #[cfg(not(any(unix, windows)))]
            let tid: u64 = 0;
            state.set_reg(dst, Value::from_i64(tid as i64));
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::SysMmap) => {
            let dst = read_reg(state)?;
            let addr_reg = read_reg(state)?;
            let len_reg = read_reg(state)?;
            let prot_reg = read_reg(state)?;
            let flags_reg = read_reg(state)?;
            let fd_reg = read_reg(state)?;
            let offset_reg = read_reg(state)?;

            let addr = state.get_reg(addr_reg).as_i64();
            let len = state.get_reg(len_reg).as_i64();
            let _offset = state.get_reg(offset_reg).as_i64();

            // Extract prot flags from MemProt struct object
            // MemProt { read: Bool, write: Bool, exec: Bool }
            let prot_val = state.get_reg(prot_reg);
            let prot_flags = extract_memprot_flags(state, prot_val);

            // Extract map flags from MapFlags struct object
            // MapFlags { shared: Bool, is_private: Bool, anonymous: Bool, fixed: Bool }
            let flags_val = state.get_reg(flags_reg);
            let map_flags = extract_mapflags(state, flags_val);

            // Extract fd from FileDesc newtype (Int)
            let fd_val = state.get_reg(fd_reg);
            let fd = extract_filedesc(state, fd_val);

            #[cfg(unix)]
            {
                let offset = _offset;
                // SAFETY: `mmap` is a well-defined kernel syscall. The caller
                // supplies the same arguments the AOT path would; invalid inputs
                // return `MAP_FAILED` without corrupting our process state. No
                // Rust references are dereferenced here.
                let result = unsafe {
                    libc::mmap(
                        addr as *mut libc::c_void,
                        len as libc::size_t,
                        prot_flags,
                        map_flags,
                        fd,
                        offset as libc::off_t,
                    )
                };

                if result == libc::MAP_FAILED {
                    let errno = get_platform_errno();
                    let err_obj = make_oserror_variant(state, errno)?;
                    state.set_reg(dst, err_obj);
                } else {
                    let ok_obj = make_result_ok_ptr(state, result as i64)?;
                    state.set_reg(dst, ok_obj);
                }
            }

            #[cfg(windows)]
            {
                let _ = (fd, map_flags);
                // Translate MemProt flags to Windows page protection constants
                let win_prot = memprot_to_win_protect(prot_flags);
                let alloc_type = 0x00001000u32 | 0x00002000u32; // MEM_COMMIT | MEM_RESERVE
                // SAFETY: VirtualAlloc is a well-defined Win32 API. Invalid inputs
                // return NULL without corrupting process state.
                let result = unsafe {
                    windows_sys::Win32::System::Memory::VirtualAlloc(
                        if addr == 0 { std::ptr::null() } else { addr as *const core::ffi::c_void },
                        len as usize,
                        alloc_type,
                        win_prot,
                    )
                };
                if result.is_null() {
                    let errno = get_platform_errno();
                    let err_obj = make_oserror_variant(state, errno)?;
                    state.set_reg(dst, err_obj);
                } else {
                    let ok_obj = make_result_ok_ptr(state, result as i64)?;
                    state.set_reg(dst, ok_obj);
                }
            }

            #[cfg(not(any(unix, windows)))]
            {
                let err_obj = make_oserror_variant_with_msg(state, 38, "mmap not supported on this platform")?;
                state.set_reg(dst, err_obj);
            }

            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::SysMunmap) => {
            let dst = read_reg(state)?;
            let addr_reg = read_reg(state)?;
            let len_reg = read_reg(state)?;

            let addr = state.get_reg(addr_reg).as_i64();
            let len = state.get_reg(len_reg).as_i64();

            #[cfg(unix)]
            {
                // SAFETY: `munmap` is a well-defined kernel syscall that fails
                // with a negative result on invalid inputs. No Rust references
                // are dereferenced; correctness is the caller's responsibility.
                let result = unsafe { libc::munmap(addr as *mut libc::c_void, len as libc::size_t) };

                if result < 0 {
                    let errno = get_platform_errno();
                    let err_obj = make_oserror_variant(state, errno)?;
                    state.set_reg(dst, err_obj);
                } else {
                    let ok_obj = make_result_ok_unit(state)?;
                    state.set_reg(dst, ok_obj);
                }
            }

            #[cfg(windows)]
            {
                let _ = len;
                // SAFETY: VirtualFree with MEM_RELEASE (0x00008000) is well-defined.
                // The size parameter must be 0 when using MEM_RELEASE.
                let result = unsafe {
                    windows_sys::Win32::System::Memory::VirtualFree(
                        addr as *mut core::ffi::c_void,
                        0,
                        0x00008000u32, // MEM_RELEASE
                    )
                };
                if result == 0 {
                    let errno = get_platform_errno();
                    let err_obj = make_oserror_variant(state, errno)?;
                    state.set_reg(dst, err_obj);
                } else {
                    let ok_obj = make_result_ok_unit(state)?;
                    state.set_reg(dst, ok_obj);
                }
            }

            #[cfg(not(any(unix, windows)))]
            {
                let _ = (addr, len);
                let err_obj = make_oserror_variant_with_msg(state, 38, "munmap not supported on this platform")?;
                state.set_reg(dst, err_obj);
            }

            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::SysMadvise) => {
            let dst = read_reg(state)?;
            let addr_reg = read_reg(state)?;
            let len_reg = read_reg(state)?;
            let advice_reg = read_reg(state)?;

            let _addr = state.get_reg(addr_reg).as_i64();
            let _len = state.get_reg(len_reg).as_i64();
            let _advice = state.get_reg(advice_reg).as_i64();

            #[cfg(unix)]
            {
                // SAFETY: `madvise` is a kernel syscall that validates the
                // supplied address range and returns `-1` on invalid input.
                let result = unsafe {
                    libc::madvise(_addr as *mut libc::c_void, _len as libc::size_t, _advice as i32)
                };

                if result < 0 {
                    let errno = get_platform_errno();
                    let err_obj = make_oserror_variant(state, errno)?;
                    state.set_reg(dst, err_obj);
                } else {
                    let ok_obj = make_result_ok_unit(state)?;
                    state.set_reg(dst, ok_obj);
                }
            }

            #[cfg(not(unix))]
            {
                // madvise is advisory-only; no-op on Windows and other platforms
                let ok_obj = make_result_ok_unit(state)?;
                state.set_reg(dst, ok_obj);
            }

            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::SysGetentropy) => {
            let dst = read_reg(state)?;
            let buf_reg = read_reg(state)?;
            let len_reg = read_reg(state)?;

            let buf = state.get_reg(buf_reg).as_i64();
            let len = state.get_reg(len_reg).as_i64();

            if len > 256 {
                // getentropy has a 256-byte limit
                let err_obj = make_oserror_variant_with_msg(state, 5, "getentropy: max 256 bytes")?;
                state.set_reg(dst, err_obj);
            } else {
                #[cfg(unix)]
                let result = {
                    // SAFETY: `len` was bounded to <= 256 above (getentropy's
                    // hard limit). `buf` is an attacker-supplied pointer — the
                    // kernel validates it and returns `-1/EFAULT` on invalid
                    // memory; this is the same contract the AOT path uses.
                    unsafe {
                        libc::getentropy(buf as *mut libc::c_void, len as libc::size_t)
                    }
                };

                #[cfg(windows)]
                let result = {
                    // SAFETY: BCryptGenRandom with BCRYPT_USE_SYSTEM_PREFERRED_RNG
                    // (flag 0x00000002) fills the buffer with cryptographic random bytes.
                    // `buf` is an address provided by the caller; `len` is bounded to <= 256.
                    let status = unsafe {
                        windows_sys::Win32::Security::Cryptography::BCryptGenRandom(
                            std::ptr::null_mut(), // BCRYPT_USE_SYSTEM_PREFERRED_RNG requires null handle
                            buf as *mut u8,
                            len as u32,
                            0x00000002u32, // BCRYPT_USE_SYSTEM_PREFERRED_RNG
                        )
                    };
                    if status == 0 { 0i32 } else { -1i32 }
                };

                #[cfg(not(any(unix, windows)))]
                let result: i32 = -1;

                if result < 0 {
                    let errno = get_platform_errno();
                    let err_obj = make_oserror_variant(state, errno)?;
                    state.set_reg(dst, err_obj);
                } else {
                    let ok_obj = make_result_ok_unit(state)?;
                    state.set_reg(dst, ok_obj);
                }
            }
            Ok(DispatchResult::Continue)
        }

        // ==============================================================
        // Symbol Resolution (0x00-0x02) — stubs
        // ==============================================================

        Some(FfiSubOpcode::LoadSymbol) => {
            // Format: dst:reg, symbol_idx:u32
            let _dst = read_reg(state)?;
            let _symbol_idx = read_u32(state)?;
            // Dynamic FFI symbol loading requires a real dlopen/dlsym runtime,
            // which the Tier 0 interpreter does not provide. Returning an
            // explicit error prevents callers from using a nil handle and
            // masking real failures as silent no-ops.
            Err(InterpreterError::NotImplemented {
                feature: "LoadSymbol: dynamic FFI loading not supported in interpreter (use AOT)",
                opcode: None,
            })
        }

        Some(FfiSubOpcode::GetLibrary) => {
            // Format: dst:reg, library_idx:u16
            let _dst = read_reg(state)?;
            let _library_idx = read_u16(state)?;
            Err(InterpreterError::NotImplemented {
                feature: "GetLibrary: dynamic library handles not supported in interpreter (use AOT)",
                opcode: None,
            })
        }

        Some(FfiSubOpcode::IsSymbolResolved) => {
            // Format: dst:reg, symbol_idx:u32
            let dst = read_reg(state)?;
            let _symbol_idx = read_u32(state)?;
            // Correct semantics: interpreter performs no dynamic linking,
            // so no symbol is ever "resolved". Callers that branch on this
            // will skip their FFI path, which is the intended behaviour.
            state.set_reg(dst, Value::from_bool(false));
            Ok(DispatchResult::Continue)
        }

        // ==============================================================
        // FFI Calling Convention Variants (0x11-0x17)
        // Route through same code path as CallFfiC (0x10)
        // ==============================================================

        Some(FfiSubOpcode::CallFfiStdcall) |
        Some(FfiSubOpcode::CallFfiSysV64) |
        Some(FfiSubOpcode::CallFfiFastcall) |
        Some(FfiSubOpcode::CallFfiAarch64) |
        Some(FfiSubOpcode::CallFfiWin64Arm64) => {
            // Same operand format as CallFfiC:
            // symbol_idx:u32, arg_count:u8, ret_reg:reg, [arg_regs...],
            // mut_ref_count:u8, [(arg_idx:u8, source_reg:reg)...]
            let symbol_idx = read_u32(state)?;
            let arg_count = read_u8(state)? as usize;
            let ret_reg = read_reg(state)?;

            let mut arg_regs = Vec::with_capacity(arg_count);
            for _ in 0..arg_count {
                arg_regs.push(read_reg(state)?);
            }

            let mut_ref_count = read_u8(state)? as usize;
            let mut source_reg_map = std::collections::HashMap::new();
            for _ in 0..mut_ref_count {
                let arg_idx = read_u8(state)?;
                let source_reg = read_reg(state)?;
                source_reg_map.insert(arg_idx, source_reg.0);
            }

            let args: Vec<Value> = arg_regs.iter().map(|r| state.get_reg(*r)).collect();

            #[cfg(feature = "ffi")]
            {
                let symbol_id = FfiSymbolId(symbol_idx);
                let module = state.module.clone();

                state.setup_callback_handler();

                let ffi_runtime = state.get_or_create_ffi_runtime()?;
                ffi_runtime.load_module_libraries(&module)
                    .map_err(|e| InterpreterError::FfiRuntimeError(format!("{}", e)))?;

                let mut ret_value = Value::nil();
                // SAFETY: See CallFfiC for the full justification — this
                // branch routes all non-C calling conventions through the
                // same FFI runtime entrypoint with identical contract.
                let writebacks = unsafe {
                    ffi_runtime.call_module_ffi_c_with_writeback_v2(
                        &module,
                        symbol_id,
                        &args,
                        &source_reg_map,
                        &mut ret_value,
                    )
                }.map_err(|e| InterpreterError::FfiRuntimeError(format!("{}", e)))?;

                // If return type is a struct, convert raw C buffer to Verum heap object
                if let Some(sym) = module.get_ffi_symbol(symbol_id)
                    && matches!(sym.signature.return_type, crate::module::CType::StructValue)
                        && let Some(layout_idx) = sym.signature.return_layout_idx {
                            let layout = &module.ffi_layouts[layout_idx as usize];
                            let c_buf_ptr = ret_value.as_ptr::<u8>();
                            if !c_buf_ptr.is_null() {
                                let max_field_idx = layout.fields.iter()
                                    .map(|f| f.name.0 as usize)
                                    .max()
                                    .unwrap_or(0);
                                let slot_count = max_field_idx + 1;
                                let type_id = layout.verum_type.unwrap_or(crate::types::TypeId::UNIT);
                                let obj = state.heap.alloc(type_id, slot_count * std::mem::size_of::<Value>())?;
                                state.record_allocation();

                                // SAFETY: See CallFfiC struct-return branch
                                // for the full justification — same layout
                                // and alloc invariants apply here.
                                let data_ptr = unsafe {
                                    (obj.as_ptr() as *mut u8).add(super::super::super::heap::OBJECT_HEADER_SIZE) as *mut Value
                                };
                                for field in &layout.fields {
                                    // SAFETY: Offsets come from the compiler
                                    // layout table; see CallFfiC branch.
                                    let c_field_ptr = unsafe { c_buf_ptr.add(field.offset as usize) };
                                    // SAFETY: `field.c_type` matches the
                                    // bytes at `c_field_ptr`; see CallFfiC.
                                    if let Some(val) = unsafe { crate::ffi::runtime::marshal_field_from_c(field.c_type, c_field_ptr) } {
                                        let slot_idx = field.name.0 as usize;
                                        // SAFETY: `slot_idx <= max_field_idx`
                                        // and `data_ptr` has `slot_count`
                                        // Value slots; in bounds.
                                        unsafe { *data_ptr.add(slot_idx) = val; }
                                    }
                                }
                                // SAFETY: Undoes the `Box::into_raw` of a
                                // [0u8; 256] performed by the FFI runtime;
                                // see CallFfiC for details.
                                unsafe { let _ = Box::from_raw(std::ptr::slice_from_raw_parts_mut(c_buf_ptr, 256)); }
                                ret_value = Value::from_ptr(obj.as_ptr() as *mut u8);
                            }
                        }

                state.set_reg(ret_reg, ret_value);

                for (reg_idx, new_val) in writebacks {
                    state.set_reg(Reg(reg_idx), new_val);
                }

                for buf in state.ffi_array_buffers.drain(..) {
                    if buf.is_mutable && !buf.buffer.is_null() && !buf.array_obj_ptr.is_null() {
                        // SAFETY: See CallFfiC writeback loop — same invariants.
                        let header = unsafe { &*(buf.array_obj_ptr as *const super::super::super::heap::ObjectHeader) };

                        if header.type_id == TypeId::U16 || header.type_id == TypeId::U32 || header.type_id == TypeId::U64 {
                            let elem_size = match header.type_id {
                                t if t == TypeId::U16 => 2usize,
                                t if t == TypeId::U32 => 4usize,
                                _ => 8usize,
                            };
                            let total_bytes = buf.count.checked_mul(elem_size).unwrap_or(0);
                            // SAFETY: Same as CallFfiC typed-array writeback.
                            let dst = unsafe { buf.array_obj_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE) };
                            // SAFETY: `buf.buffer` has `total_bytes` live bytes
                            // and `dst` has matching capacity in the heap
                            // object (disjoint allocations).
                            unsafe {
                                std::ptr::copy_nonoverlapping(buf.buffer, dst, total_bytes);
                            }
                        } else {
                            for i in 0..buf.count {
                                // SAFETY: See CallFfiC per-element loop.
                                let new_val = unsafe {
                                    match buf.element_type {
                                        0x01 => Value::from_i64(*(buf.buffer.add(i) as *const i8) as i64),
                                        0x02 => Value::from_i64(*(buf.buffer.add(i * 2) as *const i16) as i64),
                                        0x03 => Value::from_i64(*(buf.buffer.add(i * 4) as *const i32) as i64),
                                        0x04 => Value::from_i64(*(buf.buffer.add(i * 8) as *const i64)),
                                        0x05 => Value::from_f64(*(buf.buffer.add(i * 4) as *const f32) as f64),
                                        0x06 => Value::from_f64(*(buf.buffer.add(i * 8) as *const f64)),
                                        _ => Value::from_i64(*(buf.buffer.add(i * 8) as *const i64)),
                                    }
                                };
                                let elem_offset = i * std::mem::size_of::<Value>();
                                let elem_ptr = if header.type_id == TypeId::LIST {
                                    // SAFETY: See CallFfiC LIST writeback for layout details.
                                    let data_ptr = unsafe { buf.array_obj_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE) as *const Value };
                                    // SAFETY: Slot 2 holds the backing buffer pointer.
                                    let backing = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
                                    // SAFETY: `elem_offset` is in bounds per `i < buf.count`.
                                    unsafe { backing.add(super::super::super::heap::OBJECT_HEADER_SIZE + elem_offset) as *mut Value }
                                } else {
                                    // SAFETY: Non-LIST arrays pack Values after header.
                                    unsafe { buf.array_obj_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE + elem_offset) as *mut Value }
                                };
                                // SAFETY: `elem_ptr` is aligned and writable.
                                unsafe { *elem_ptr = new_val; }
                            }
                        }
                    }
                }

                state.teardown_callback_handler();
            }
            #[cfg(not(feature = "ffi"))]
            {
                let _ = (symbol_idx, args, source_reg_map);
                return Err(InterpreterError::NotImplemented {
                    feature: "FFI calls require the 'ffi' feature",
                    opcode: None,
                });
            }

            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::CallFfiVariadic) => {
            // Format: symbol_idx:u32, fixed_count:u8, variadic_count:u8, ret_reg:reg, [arg_regs...],
            //         mut_ref_count:u8, [(arg_idx:u8, source_reg:reg)...]
            let symbol_idx = read_u32(state)?;
            let fixed_count = read_u8(state)? as usize;
            let variadic_count = read_u8(state)? as usize;
            let arg_count = fixed_count + variadic_count;
            let ret_reg = read_reg(state)?;

            let mut arg_regs = Vec::with_capacity(arg_count);
            for _ in 0..arg_count {
                arg_regs.push(read_reg(state)?);
            }

            let mut_ref_count = read_u8(state)? as usize;
            let mut source_reg_map = std::collections::HashMap::new();
            for _ in 0..mut_ref_count {
                let arg_idx = read_u8(state)?;
                let source_reg = read_reg(state)?;
                source_reg_map.insert(arg_idx, source_reg.0);
            }

            let args: Vec<Value> = arg_regs.iter().map(|r| state.get_reg(*r)).collect();

            #[cfg(feature = "ffi")]
            {
                let symbol_id = FfiSymbolId(symbol_idx);
                let module = state.module.clone();

                state.setup_callback_handler();

                let ffi_runtime = state.get_or_create_ffi_runtime()?;
                ffi_runtime.load_module_libraries(&module)
                    .map_err(|e| InterpreterError::FfiRuntimeError(format!("{}", e)))?;

                let mut ret_value = Value::nil();
                // SAFETY: See CallFfiC — same FFI runtime, same contract.
                let writebacks = unsafe {
                    ffi_runtime.call_module_ffi_c_with_writeback_v2(
                        &module,
                        symbol_id,
                        &args,
                        &source_reg_map,
                        &mut ret_value,
                    )
                }.map_err(|e| InterpreterError::FfiRuntimeError(format!("{}", e)))?;

                state.set_reg(ret_reg, ret_value);

                for (reg_idx, new_val) in writebacks {
                    state.set_reg(Reg(reg_idx), new_val);
                }

                for buf in state.ffi_array_buffers.drain(..) {
                    if buf.is_mutable && !buf.buffer.is_null() && !buf.array_obj_ptr.is_null() {
                        // SAFETY: See CallFfiC writeback loop.
                        let header = unsafe { &*(buf.array_obj_ptr as *const super::super::super::heap::ObjectHeader) };
                        if header.type_id == TypeId::U16 || header.type_id == TypeId::U32 || header.type_id == TypeId::U64 {
                            let elem_size = match header.type_id {
                                t if t == TypeId::U16 => 2usize,
                                t if t == TypeId::U32 => 4usize,
                                _ => 8usize,
                            };
                            let total_bytes = buf.count.checked_mul(elem_size).unwrap_or(0);
                            // SAFETY: See CallFfiC typed-array writeback.
                            let dst = unsafe { buf.array_obj_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE) };
                            // SAFETY: Source/dest both live; total_bytes in bounds.
                            unsafe { std::ptr::copy_nonoverlapping(buf.buffer, dst, total_bytes); }
                        } else {
                            for i in 0..buf.count {
                                // SAFETY: See CallFfiC per-element reader.
                                let new_val = unsafe {
                                    match buf.element_type {
                                        0x01 => Value::from_i64(*(buf.buffer.add(i) as *const i8) as i64),
                                        0x02 => Value::from_i64(*(buf.buffer.add(i * 2) as *const i16) as i64),
                                        0x03 => Value::from_i64(*(buf.buffer.add(i * 4) as *const i32) as i64),
                                        0x04 => Value::from_i64(*(buf.buffer.add(i * 8) as *const i64)),
                                        0x05 => Value::from_f64(*(buf.buffer.add(i * 4) as *const f32) as f64),
                                        0x06 => Value::from_f64(*(buf.buffer.add(i * 8) as *const f64)),
                                        _ => Value::from_i64(*(buf.buffer.add(i * 8) as *const i64)),
                                    }
                                };
                                let elem_offset = i * std::mem::size_of::<Value>();
                                let elem_ptr = if header.type_id == TypeId::LIST {
                                    // SAFETY: See CallFfiC LIST writeback.
                                    let data_ptr = unsafe { buf.array_obj_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE) as *const Value };
                                    // SAFETY: Slot 2 = backing pointer.
                                    let backing = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
                                    // SAFETY: Offset in bounds per `i < buf.count`.
                                    unsafe { backing.add(super::super::super::heap::OBJECT_HEADER_SIZE + elem_offset) as *mut Value }
                                } else {
                                    // SAFETY: Non-LIST arrays pack Values after header.
                                    unsafe { buf.array_obj_ptr.add(super::super::super::heap::OBJECT_HEADER_SIZE + elem_offset) as *mut Value }
                                };
                                // SAFETY: `elem_ptr` is aligned and writable.
                                unsafe { *elem_ptr = new_val; }
                            }
                        }
                    }
                }

                state.teardown_callback_handler();
            }
            #[cfg(not(feature = "ffi"))]
            {
                let _ = (symbol_idx, fixed_count, variadic_count, args, source_reg_map);
                return Err(InterpreterError::NotImplemented {
                    feature: "FFI calls require the 'ffi' feature",
                    opcode: None,
                });
            }

            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::CallFfiIndirect) => {
            // Format: ptr_reg:reg, signature_idx:u32, arg_count:u8, ret_reg:reg, [arg_regs...],
            //         mut_ref_count:u8, [(arg_idx:u8, source_reg:reg)...]
            let _ptr_reg = read_reg(state)?;
            let _signature_idx = read_u32(state)?;
            let arg_count = read_u8(state)? as usize;
            let ret_reg = read_reg(state)?;

            // Consume argument registers to keep bytecode pointer in sync
            for _ in 0..arg_count {
                let _ = read_reg(state)?;
            }

            let mut_ref_count = read_u8(state)? as usize;
            for _ in 0..mut_ref_count {
                let _ = read_u8(state)?;
                let _ = read_reg(state)?;
            }

            // Indirect FFI calls not supported in interpreter
            state.set_reg(ret_reg, Value::nil());
            Ok(DispatchResult::Continue)
        }

        // ==============================================================
        // Marshalling (0x20-0x27) — stubs (pass values through)
        // ==============================================================

        Some(FfiSubOpcode::MarshalToC) | Some(FfiSubOpcode::MarshalFromC) => {
            // Format: dst:reg, src:reg, c_type:u8
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let _c_type = read_u8(state)?;
            // In interpreter, values are already in Verum representation — pass through
            state.set_reg(dst, state.get_reg(src));
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::StringToC) => {
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            // Interpreter doesn't need C null-termination — return string value as-is
            state.set_reg(dst, state.get_reg(src));
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::StringFromC) => {
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            // Pass C string value through as Verum text
            state.set_reg(dst, state.get_reg(src));
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::ArrayFromC) => {
            // Format: dst:reg, ptr:reg, len:reg, elem_type:u8
            let _dst = read_reg(state)?;
            let _src = read_reg(state)?;
            let _len = read_reg(state)?;
            let _elem_type = read_u8(state)?;
            // Marshalling a raw C array into a Verum List requires building
            // a proper List<T> from unmarshalled bytes — passing the pointer
            // through unchanged would crash on indexing. Return explicit
            // error so the caller sees the unsupported operation rather
            // than corrupting memory via a fake list value.
            Err(InterpreterError::NotImplemented {
                feature: "ArrayFromC: array marshalling not supported in interpreter (use AOT)",
                opcode: None,
            })
        }

        Some(FfiSubOpcode::StructToC) | Some(FfiSubOpcode::StructFromC) => {
            // Format: dst:reg, src:reg, layout_idx:u32
            let _dst = read_reg(state)?;
            let _src = read_reg(state)?;
            let _layout_idx = read_u32(state)?;
            // Struct layout conversion between Verum records and C layouts
            // requires ABI-aware field reordering and padding. Passing the
            // value through unchanged would produce C structs with the wrong
            // memory layout. Return explicit error.
            Err(InterpreterError::NotImplemented {
                feature: "StructToC/FromC: struct marshalling not supported in interpreter (use AOT)",
                opcode: None,
            })
        }

        // ==============================================================
        // CRealloc (0x42)
        // ==============================================================

        Some(FfiSubOpcode::CRealloc) => {
            // Format: dst:reg, ptr:reg, size:reg
            let dst = read_reg(state)?;
            let _ptr_reg = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let size = state.get_reg(size_reg).as_i64() as usize;
            if size > 0 {
                let layout = std::alloc::Layout::from_size_align(size, 8)
                    .map_err(|_| InterpreterError::InvalidOperand {
                        message: format!("invalid reallocation size: {}", size),
                    })?;
                // SAFETY: `size > 0` is checked above and `layout` has a
                // valid alignment, so `alloc_zeroed`'s precondition holds.
                // A null return is tolerated by `Value::from_ptr`.
                let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
                state.set_reg(dst, Value::from_ptr(ptr));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        // ==============================================================
        // FreeCallback (0x51) — no-op
        // ==============================================================

        Some(FfiSubOpcode::FreeCallback) => {
            // Format: trampoline:reg
            let _callback_reg = read_reg(state)?;
            // No-op: callback trampolines are cleaned up on interpreter exit
            Ok(DispatchResult::Continue)
        }

        // ==============================================================
        // Mach Kernel Operations (0x90-0x98) — macOS stubs
        // ==============================================================

        Some(FfiSubOpcode::MachVmAllocate) => {
            // Format: dst:reg, size:reg, anywhere:reg
            let dst = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let _anywhere_reg = read_reg(state)?;
            let size = state.get_reg(size_reg).as_i64() as usize;
            if size > 0 {
                let layout = std::alloc::Layout::from_size_align(size, 4096)
                    .unwrap_or(std::alloc::Layout::new::<u8>());
                // SAFETY: `size > 0` is checked above; the layout has a
                // valid alignment (4096 or `Layout::new::<u8>()`). Null
                // returns are tolerated by `Value::from_ptr`.
                let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
                state.set_reg(dst, Value::from_ptr(ptr));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::MachVmDeallocate) => {
            // Format: dst:reg, addr:reg, size:reg
            let _dst = read_reg(state)?;
            let _addr_reg = read_reg(state)?;
            let _size_reg = read_reg(state)?;
            // No-op: cannot safely deallocate without knowing exact layout
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::MachVmProtect) => {
            // Format: dst:reg, addr:reg, size:reg, prot:reg
            let _dst = read_reg(state)?;
            let _addr_reg = read_reg(state)?;
            let _size_reg = read_reg(state)?;
            let _prot_reg = read_reg(state)?;
            // No-op: memory protection stubs in interpreter
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::MachSemCreate) => {
            // Format: dst:reg, initial_value:reg
            let dst = read_reg(state)?;
            let _value_reg = read_reg(state)?;
            // Return a fake semaphore handle (incrementing ID)
            static NEXT_SEM: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);
            let id = NEXT_SEM.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            state.set_reg(dst, Value::from_i64(id));
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::MachSemDestroy) |
        Some(FfiSubOpcode::MachSemSignal) |
        Some(FfiSubOpcode::MachSemWait) => {
            // Format: dst:reg, sem:reg
            let _dst = read_reg(state)?;
            let _sem_reg = read_reg(state)?;
            // No-op: semaphore operations are stubs in interpreter
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::MachErrorString) => {
            // Format: dst:reg, kern_return:reg
            let dst = read_reg(state)?;
            let _err_reg = read_reg(state)?;
            // Return "success" string for any error code
            let val = alloc_string_value(state, "success")?;
            state.set_reg(dst, val);
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::MachSleepUntil) => {
            // Format: dst:reg, deadline:reg
            let _dst = read_reg(state)?;
            let _deadline_reg = read_reg(state)?;
            // No-op: sleep stubs in interpreter
            Ok(DispatchResult::Continue)
        }

        // =================================================================
        // CBGR Memory Operations (0xA0-0xA2) — tracked allocation for
        // Shared<T>, Heap<T>, List/Map internals, and any code path that
        // calls the stdlib `cbgr_alloc(size, align)` / `cbgr_dealloc`.
        //
        // In the Tier 0 interpreter we bypass the stdlib allocator
        // (allocator.vr -> os_mmap -> FFI mmap) entirely and allocate
        // through Rust's std allocator, then register the allocation in
        // `state.cbgr_allocations` so that subsequent CBGR validation
        // ops (ChkRef, GetGeneration, etc.) see a live allocation.
        //
        // The result is a tuple `(ptr, generation, epoch)` matching the
        // shape of `AllocResult` in `core/mem/allocator.vr`. We package it
        // as a heap-allocated tuple object so pattern-matching
        // `Ok((ptr, gen, epoch))` continues to work unchanged in user
        // code.
        // =================================================================
        Some(FfiSubOpcode::CbgrAlloc) | Some(FfiSubOpcode::CbgrAllocZeroed) => {
            let zeroed = matches!(sub_op, Some(FfiSubOpcode::CbgrAllocZeroed));
            let dst = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let align_reg = read_reg(state)?;
            let raw_size = state.get_reg(size_reg).as_integer_compatible();
            let raw_align = state.get_reg(align_reg).as_integer_compatible();
            // Size/align arguments arrive from the Verum side and can be
            // garbage (bad codegen, pointer-tagged values leaking into
            // the call). Reject absurd sizes/aligns up front and return
            // the same "Err(OutOfMemory)" shape the stdlib would — the
            // caller can then surface an allocation failure instead of
            // the interpreter panicking on LayoutError.
            const MAX_ALLOC: i64 = 1 << 30; // 1 GiB cap in the interpreter
            if raw_size <= 0 || raw_size > MAX_ALLOC || raw_align <= 0 || raw_align > 4096 {
                // Build an Err variant with a nil payload so destructuring
                // `Err(_)` fires cleanly in user code.
                let variant_size = 8 + std::mem::size_of::<Value>();
                let err_obj = state.heap.alloc_with_init(
                    crate::types::TypeId(0x8000 + 1), // tag=1 = Err
                    variant_size,
                    |data| {
                        let tag_ptr = data.as_mut_ptr() as *mut u32;
                        unsafe { *tag_ptr = 1; *tag_ptr.add(1) = 1; }
                    },
                )?;
                let err_base = err_obj.as_ptr() as *mut u8;
                let payload_off = crate::interpreter::heap::OBJECT_HEADER_SIZE + 8;
                unsafe {
                    std::ptr::write(err_base.add(payload_off) as *mut Value, Value::nil());
                }
                state.set_reg(dst, Value::from_ptr(err_obj.as_ptr()));
                return Ok(DispatchResult::Continue);
            }
            let size = raw_size as usize;
            let align: usize = (raw_align as usize).next_power_of_two().max(8);
            let layout = std::alloc::Layout::from_size_align(size, align)
                .unwrap_or_else(|_| std::alloc::Layout::from_size_align(size, 8).unwrap());
            // SAFETY: layout has non-zero size and valid alignment by
            // construction. The allocator contract requires matching the
            // layout on dealloc; callers are expected to route through
            // CbgrDealloc (which currently leaks — see below).
            let ptr = unsafe {
                if zeroed { std::alloc::alloc_zeroed(layout) } else { std::alloc::alloc(layout) }
            };
            if ptr.is_null() {
                // Model `AllocError::OutOfMemory` as returning a nil Value
                // so pattern-match `Err(e) => ...` fires. The tuple shape
                // matching below is only materialised on success.
                state.set_reg(dst, Value::nil());
                return Ok(DispatchResult::Continue);
            }
            // Track so CBGR validation ops see the allocation.
            state.cbgr_allocations.insert(ptr as usize);
            // Build (ptr, generation, epoch) tuple. Fresh generation = 1
            // (0 is reserved "unallocated" in CBGR conventions), fresh
            // epoch = current interpreter epoch counter.
            let generation = 1i64;
            let epoch = state.cbgr_epoch as i64;
            // Materialise a 3-tuple matching `Pack` layout so
            // `let (ptr, g, e) = …` destructures each field at its
            // expected offset.
            let tuple_size = 3 * std::mem::size_of::<Value>();
            let tuple_obj = state.heap.alloc_with_init(
                crate::types::TypeId::TUPLE,
                tuple_size,
                |_data| {},
            )?;
            let tuple_data = tuple_obj.data_ptr() as *mut Value;
            unsafe {
                std::ptr::write(tuple_data.add(0), Value::from_i64(ptr as i64));
                std::ptr::write(tuple_data.add(1), Value::from_i64(generation));
                std::ptr::write(tuple_data.add(2), Value::from_i64(epoch));
            }
            let tuple_val = Value::from_ptr(tuple_obj.as_ptr());

            // Wrap in Ok(tuple). Result layout (from `handle_make_variant`):
            //   [tag: u32][field_count: u32][payload: Value * N]
            // Ok = tag 0, single payload field holding the tuple.
            let variant_size = 8 + std::mem::size_of::<Value>();
            let variant_obj = state.heap.alloc_with_init(
                crate::types::TypeId(0x8000), // tag=0 = Ok (variant TypeId base | tag)
                variant_size,
                |data| {
                    let tag_ptr = data.as_mut_ptr() as *mut u32;
                    unsafe {
                        *tag_ptr = 0;
                        *tag_ptr.add(1) = 1;
                    }
                },
            )?;
            let variant_base = variant_obj.as_ptr() as *mut u8;
            let payload_offset = crate::interpreter::heap::OBJECT_HEADER_SIZE + 8;
            unsafe {
                let field_ptr = variant_base.add(payload_offset) as *mut Value;
                std::ptr::write(field_ptr, tuple_val);
            }
            state.set_reg(dst, Value::from_ptr(variant_obj.as_ptr()));
            Ok(DispatchResult::Continue)
        }

        Some(FfiSubOpcode::CbgrDealloc) => {
            let _dst = read_reg(state)?;
            let _ptr_reg = read_reg(state)?;
            let _size_reg = read_reg(state)?;
            let _align_reg = read_reg(state)?;
            // Intentional leak in the interpreter: the stdlib tracks
            // Shared refcount / drop ordering at a level above us, but
            // the interpreter has no way to match the exact Layout
            // passed at allocation time without carrying extra metadata.
            // Preferring leak over double-free matches __dealloc_raw.
            Ok(DispatchResult::Continue)
        }

        // Unimplemented sub-opcodes
        _ => {
            Err(InterpreterError::NotImplemented {
                feature: "ffi_extended sub-opcode",
                opcode: None,
            })
        }
    }
}

// ==============================================================
// System Call Helper Functions
// ==============================================================

/// Extract MemProt flags from a struct object.
/// MemProt layout: ObjectHeader + [read: Bool, write: Bool, exec: Bool]
///
/// On Unix, returns POSIX PROT_* flags. On other platforms, returns a
/// platform-neutral bitmask (read=1, write=2, exec=4) that callers
/// translate to platform-specific constants.
pub(in super::super) fn extract_memprot_flags(_state: &InterpreterState, val: Value) -> i32 {
    // Platform-neutral protection flag constants (match POSIX values)
    const PROT_READ: i32 = 1;
    const PROT_WRITE: i32 = 2;
    const PROT_EXEC: i32 = 4;

    let mut flags: i32 = 0;
    if val.is_ptr() && !val.is_nil() {
        let ptr = val.as_ptr::<u8>();
        if !ptr.is_null() {
            // SAFETY: `val.is_ptr()` and `!ptr.is_null()` have both been
            // checked; all VBC pointer-tagged Values originate from
            // `state.heap.alloc()`, so the object starts with ObjectHeader.
            let header = unsafe { &*(ptr as *const super::super::super::heap::ObjectHeader) };
            let header_size = super::super::super::heap::OBJECT_HEADER_SIZE;
            let val_size = std::mem::size_of::<Value>();
            let slot_count = header.size as usize / val_size;

            // Scan for Bool fields in declaration order.
            // MemProt fields: read, write, exec (all Bool, interned consecutively).
            // Fields are stored at their globally-interned indices, so we scan all slots.
            let flag_map: &[i32] = &[PROT_READ, PROT_WRITE, PROT_EXEC];
            let mut bool_idx = 0;
            for i in 0..slot_count {
                // SAFETY: `i < slot_count`, and `slot_count = header.size /
                // val_size`, so `header_size + i * val_size < header_size +
                // header.size`. That lies within the allocation. The Value
                // is aligned (Value = 8 bytes, matches its own alignment).
                let field_val = unsafe { *(ptr.add(header_size + i * val_size) as *const Value) };
                if field_val.is_bool() {
                    if bool_idx < flag_map.len() && field_val.as_bool() {
                        flags |= flag_map[bool_idx];
                    }
                    bool_idx += 1;
                }
            }
        }
    } else {
        // Scalar: treat as raw int flags
        flags = val.as_i64() as i32;
    }
    flags
}

/// Extract MapFlags flags from a struct object.
/// MapFlags layout: ObjectHeader + [shared: Bool, is_private: Bool, anonymous: Bool, fixed: Bool]
///
/// Returns platform-neutral bitmask flags. On Unix, these match POSIX MAP_*
/// constants. Callers on other platforms translate as needed.
pub(in super::super) fn extract_mapflags(_state: &InterpreterState, val: Value) -> i32 {
    // Platform-neutral map flag constants (match POSIX values on Linux)
    const MAP_SHARED: i32 = 0x01;
    const MAP_PRIVATE: i32 = 0x02;
    #[cfg(target_os = "linux")]
    const MAP_ANON: i32 = 0x20;
    #[cfg(target_os = "macos")]
    const MAP_ANON: i32 = 0x1000;
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    const MAP_ANON: i32 = 0x20; // Default to Linux value
    const MAP_FIXED: i32 = 0x10;

    let mut flags: i32 = 0;
    if val.is_ptr() && !val.is_nil() {
        let ptr = val.as_ptr::<u8>();
        if !ptr.is_null() {
            // SAFETY: See extract_memprot_flags — same invariants.
            let header = unsafe { &*(ptr as *const super::super::super::heap::ObjectHeader) };
            let header_size = super::super::super::heap::OBJECT_HEADER_SIZE;
            let val_size = std::mem::size_of::<Value>();
            let slot_count = header.size as usize / val_size;

            // Scan for Bool fields in declaration order.
            // MapFlags fields: shared, is_private, anonymous, fixed (all Bool).
            let flag_map: &[i32] = &[MAP_SHARED, MAP_PRIVATE, MAP_ANON, MAP_FIXED];
            let mut bool_idx = 0;
            for i in 0..slot_count {
                // SAFETY: See extract_memprot_flags — in bounds, aligned.
                let field_val = unsafe { *(ptr.add(header_size + i * val_size) as *const Value) };
                if field_val.is_bool() {
                    if bool_idx < flag_map.len() && field_val.as_bool() {
                        flags |= flag_map[bool_idx];
                    }
                    bool_idx += 1;
                }
            }
        }
    } else {
        // Scalar: treat as raw int flags
        flags = val.as_i64() as i32;
    }
    flags
}

/// Extract file descriptor integer from a FileDesc newtype or variant.
/// FileDesc is a newtype (Int) — may be stored as a variant with tag or directly as Int.
pub(in super::super) fn extract_filedesc(state: &InterpreterState, val: Value) -> i32 {
    let _ = state;
    if val.is_ptr() && !val.is_nil() {
        let ptr = val.as_ptr::<u8>();
        if !ptr.is_null() {
            let header_size = super::super::super::heap::OBJECT_HEADER_SIZE;
            // Check if variant (type_id >= 0x8000)
            // SAFETY: `val.is_ptr()` and `!ptr.is_null()` checked above;
            // the pointer refers to a live heap object with ObjectHeader
            // as its prefix.
            let header = unsafe { &*(ptr as *const super::super::super::heap::ObjectHeader) };
            if header.type_id.0 >= 0x8000 {
                // Variant: extract payload[0]
                let payload_offset = header_size + 8; // skip tag + padding
                // SAFETY: Variant objects are laid out as [header, tag:u64,
                // payload...] — the payload lies at `header_size + 8` and
                // the first Value is initialized at construction time.
                let inner = unsafe { *(ptr.add(payload_offset) as *const Value) };
                return inner.as_i64() as i32;
            }
            // Record: field 0 is the inner Int
            // SAFETY: Records place their first field immediately after the
            // header; field 0 is always present for a FileDesc newtype.
            let inner = unsafe { *(ptr.add(header_size) as *const Value) };
            return inner.as_i64() as i32;
        }
    }
    // Scalar int
    val.as_i64() as i32
}

#[cfg(feature = "ffi")]
/// Apply error protocol checking to an FFI return value.
///
/// Mirrors the LLVM lowering's `emit_ffi_error_protocol_check` for differential
/// correctness between interpreter (Tier 0) and AOT (Tier 1).
///
/// Convention: for protocols that detect errors, the return value is negated errno
/// (negative i64) on error, or the raw return value on success.
fn apply_error_protocol(
    state: &mut InterpreterState,
    sym: &crate::module::FfiSymbol,
    ret_value: Value,
) -> InterpreterResult<Value> {
    use crate::module::ErrorProtocol;

    match sym.error_protocol {
        ErrorProtocol::None | ErrorProtocol::Exception | ErrorProtocol::_ReservedOutError => {
            Ok(ret_value)
        }

        ErrorProtocol::NegOneErrno => {
            let ret_i64 = ret_value.as_i64();
            if ret_i64 == -1 {
                let errno = read_errno(state);
                Ok(Value::from_i64(-(errno as i64)))
            } else {
                Ok(ret_value)
            }
        }

        ErrorProtocol::NullErrno => {
            let ptr = ret_value.as_i64();
            if ptr == 0 {
                let errno = read_errno(state);
                Ok(Value::from_i64(-(errno as i64)))
            } else {
                Ok(ret_value)
            }
        }

        ErrorProtocol::ZeroSuccess => {
            let ret_i64 = ret_value.as_i64();
            if ret_i64 != 0 {
                Ok(Value::from_i64(-ret_i64))
            } else {
                Ok(Value::from_i64(0))
            }
        }

        ErrorProtocol::HResult => {
            let ret_i64 = ret_value.as_i64();
            if ret_i64 < 0 {
                Ok(Value::from_i64(ret_i64))
            } else {
                Ok(Value::from_i64(0))
            }
        }

        ErrorProtocol::ReturnCodePattern => {
            let ret_i64 = ret_value.as_i64();
            if ret_i64 == sym.error_sentinel {
                let errno = read_errno(state);
                Ok(Value::from_i64(-(errno as i64)))
            } else {
                Ok(ret_value)
            }
        }

        ErrorProtocol::SentinelWithErrno => {
            let ptr = ret_value.as_i64();
            if ptr == sym.error_sentinel {
                let errno = read_errno(state);
                Ok(Value::from_i64(-(errno as i64)))
            } else {
                Ok(ret_value)
            }
        }
    }
}

#[cfg(feature = "ffi")]
/// Read errno from the FFI runtime.
#[inline]
fn read_errno(state: &mut InterpreterState) -> i32 {
    match state.get_or_create_ffi_runtime() {
        Ok(rt) => rt.get_errno(),
        Err(_) => 0,
    }
}

/// Construct a Result::Err(OSError { code, message }) variant.
/// Result::Err has tag=1, field_count=1 (the OSError).
/// OSError is a record with fields: code: Int, message: Text.
pub(in super::super) fn make_oserror_variant(state: &mut InterpreterState, errno: i32) -> InterpreterResult<Value> {
    let msg = errno_to_string(errno);
    make_oserror_variant_with_msg(state, errno, &msg)
}

/// Construct a Result::Err(OSError { code, message }) variant with custom message.
pub(in super::super) fn make_oserror_variant_with_msg(state: &mut InterpreterState, code: i32, msg: &str) -> InterpreterResult<Value> {
    use crate::types::TypeId;

    // Create the message string value first (before borrowing heap in alloc_with_init)
    let msg_val = if let Some(small) = Value::from_small_string(msg) {
        small
    } else {
        // Allocate heap string: [len: u64][bytes...]
        let bytes = msg.as_bytes();
        let len = bytes.len();
        let alloc_size = 8 + len;
        let str_obj = state.heap.alloc(crate::types::TypeId(0x0001), alloc_size)?;
        state.record_allocation();
        let base_ptr = str_obj.as_ptr() as *mut u8;
        // SAFETY: `str_obj` was just returned from `alloc` with `alloc_size`
        // bytes of data storage (`8 + len`), so we have 8 bytes for the
        // length prefix and `len` bytes for the UTF-8 payload. `bytes` is a
        // live slice produced by `msg.as_bytes()` that does not overlap the
        // fresh allocation.
        unsafe {
            let data_offset = super::super::super::heap::OBJECT_HEADER_SIZE;
            let len_ptr = base_ptr.add(data_offset) as *mut u64;
            *len_ptr = len as u64;
            let bytes_ptr = base_ptr.add(data_offset + 8);
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), bytes_ptr, len);
        }
        Value::from_ptr(base_ptr)
    };

    let code_val = Value::from_i64(code as i64);

    // Allocate the OSError record: { code: Int, message: Text }
    let os_error_size = 2 * std::mem::size_of::<Value>(); // 2 fields
    let os_error_obj = state.heap.alloc_with_init(
        TypeId(TypeId::FIRST_USER + 100), // OS error type
        os_error_size,
        |data| {
            let ptr = data.as_mut_ptr();
            // SAFETY: `data` is a `&mut [u8]` of exactly `os_error_size`
            // bytes (2 * sizeof(Value)) provided by `alloc_with_init`; two
            // Value writes of 8 bytes each fit in-bounds and are aligned
            // (the heap returns 8-byte-aligned data).
            unsafe {
                // Field 0: code (Int)
                std::ptr::write(ptr as *mut Value, code_val);
                // Field 1: message (Text)
                std::ptr::write((ptr as *mut Value).add(1), msg_val);
            }
        },
    )?;
    state.record_allocation();

    // Now allocate the Result::Err variant (tag=1, field_count=1, payload=OSError)
    let variant_data_size = 8 + std::mem::size_of::<Value>(); // tag:u32 + padding:u32 + 1 payload
    let variant_obj = state.heap.alloc_with_init(
        TypeId(0x8000 + 1), // tag 1 = Err
        variant_data_size,
        |data| {
            let ptr = data.as_mut_ptr();
            // SAFETY: `data` is a `&mut [u8]` of exactly `variant_data_size`
            // bytes. We write a u32 tag + u32 field_count (8 bytes) followed
            // by a Value payload (8 bytes) — total 16 bytes, matches the
            // allocation. Pointer alignment is suitable: the heap returns
            // 8-byte-aligned data and u32 writes only require 4-byte
            // alignment.
            unsafe {
                // Tag
                *(ptr as *mut u32) = 1; // Err
                // Field count
                *((ptr as *mut u32).add(1)) = 1;
                // Payload[0] = OSError object pointer
                let payload_ptr = ptr.add(8) as *mut Value;
                std::ptr::write(payload_ptr, Value::from_ptr(os_error_obj.as_ptr()));
            }
        },
    )?;
    state.record_allocation();

    Ok(Value::from_ptr(variant_obj.as_ptr()))
}

/// Construct a Result::Ok(ptr_value) variant for mmap.
pub(in super::super) fn make_result_ok_ptr(state: &mut InterpreterState, ptr_val: i64) -> InterpreterResult<Value> {
    use crate::types::TypeId;

    // Result::Ok variant (tag=0, field_count=1, payload=pointer)
    let variant_data_size = 8 + std::mem::size_of::<Value>();
    let variant_obj = state.heap.alloc_with_init(
        TypeId(0x8000), // tag 0 = Ok
        variant_data_size,
        |data| {
            let ptr = data.as_mut_ptr();
            // SAFETY: See make_oserror_variant_with_msg — same variant
            // layout, same `variant_data_size` of 16 bytes.
            unsafe {
                // Tag
                *(ptr as *mut u32) = 0; // Ok
                // Field count
                *((ptr as *mut u32).add(1)) = 1;
                // Payload[0] = pointer value
                let payload_ptr = ptr.add(8) as *mut Value;
                std::ptr::write(payload_ptr, Value::from_i64(ptr_val));
            }
        },
    )?;
    state.record_allocation();

    Ok(Value::from_ptr(variant_obj.as_ptr()))
}

/// Construct a Result::Ok(()) variant.
pub(in super::super) fn make_result_ok_unit(state: &mut InterpreterState) -> InterpreterResult<Value> {
    use crate::types::TypeId;

    // Result::Ok variant (tag=0, field_count=0 for unit payload)
    let variant_data_size = 8; // tag:u32 + padding:u32, no payload
    let variant_obj = state.heap.alloc_with_init(
        TypeId(0x8000), // tag 0 = Ok
        variant_data_size,
        |data| {
            let ptr = data.as_mut_ptr();
            // SAFETY: `data` is an 8-byte slice; two u32 writes fit and are
            // 4-byte aligned (heap returns 8-aligned pointers).
            unsafe {
                *(ptr as *mut u32) = 0; // Ok
                *((ptr as *mut u32).add(1)) = 0; // field_count = 0
            }
        },
    )?;
    state.record_allocation();

    Ok(Value::from_ptr(variant_obj.as_ptr()))
}

/// Convert errno to a human-readable string.
pub(in super::super) fn errno_to_string(errno: i32) -> String {
    #[cfg(unix)]
    {
        // SAFETY: `strerror` returns either a null pointer or a pointer to a
        // static (or thread-local) NUL-terminated string owned by libc. The
        // returned pointer is valid until the next call to `strerror` on the
        // same thread, which cannot happen before the `CStr::from_ptr` read
        // below.
        let cstr = unsafe { libc::strerror(errno) };
        if cstr.is_null() {
            format!("errno {}", errno)
        } else {
            // SAFETY: Null was checked immediately above; libc guarantees the
            // pointer is a NUL-terminated C string valid for reads on this
            // thread. `to_string_lossy().into_owned()` immediately copies the
            // bytes, so the string is owned before libc can reuse the buffer.
            let c_str = unsafe { std::ffi::CStr::from_ptr(cstr) };
            c_str.to_string_lossy().into_owned()
        }
    }
    #[cfg(not(unix))]
    {
        // On Windows and other platforms, provide a generic message
        // since strerror is not available from libc.
        format!("OS error {}", errno)
    }
}

/// Get the current platform errno/last-error code.
#[inline]
fn get_platform_errno() -> i32 {
    #[cfg(unix)]
    {
        // Use std::io::Error::last_os_error() which is cross-platform within Unix
        std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
    }
    #[cfg(windows)]
    {
        // SAFETY: GetLastError is always safe to call.
        unsafe { windows_sys::Win32::Foundation::GetLastError() as i32 }
    }
    #[cfg(not(any(unix, windows)))]
    {
        0
    }
}

/// Convert platform-neutral MemProt flags (read=1, write=2, exec=4) to
/// Windows page protection constants.
#[cfg(windows)]
fn memprot_to_win_protect(prot_flags: i32) -> u32 {
    let read = prot_flags & 1 != 0;
    let write = prot_flags & 2 != 0;
    let exec = prot_flags & 4 != 0;
    match (read, write, exec) {
        (_, _, true) if write => 0x40, // PAGE_EXECUTE_READWRITE
        (_, _, true) => 0x20,          // PAGE_EXECUTE_READ
        (_, true, _) => 0x04,          // PAGE_READWRITE
        (true, _, _) => 0x02,          // PAGE_READONLY
        _ => 0x01,                     // PAGE_NOACCESS
    }
}
