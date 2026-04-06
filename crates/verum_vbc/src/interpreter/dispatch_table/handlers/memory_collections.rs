//! Memory allocation and collection instruction handlers for VBC interpreter.

use crate::types::TypeId;
use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::super::heap;
use super::super::DispatchResult;
use super::bytecode_io::*;
use super::cbgr_helpers::{is_cbgr_ref, decode_cbgr_ref};

// ============================================================================
// Memory + Collection Operations
// ============================================================================

/// New (0x60) - Allocate object with given type
pub(in super::super) fn handle_new(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let type_id_val = read_varint(state)? as u32;
    let type_id = TypeId(type_id_val);
    let field_count = read_varint(state)? as usize;

    // Allocate enough space for all fields
    let size = field_count.max(1) * std::mem::size_of::<Value>();
    let obj = state.heap.alloc(type_id, size)?;
    state.record_allocation();

    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
    Ok(DispatchResult::Continue)
}

/// NewArray (0x61 in table, but actually 0x67 in instruction.rs)
/// Encoding: dst, elem, len - allocate array: dst = [elem; len]
pub(in super::super) fn handle_new_array(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let elem = read_reg(state)?;
    let len_reg = read_reg(state)?;

    let len_val = state.get_reg(len_reg).as_i64();
    if len_val < 0 {
        return Err(InterpreterError::Panic {
            message: "negative array length".into(),
        });
    }
    let length = len_val as usize;
    // Cap array length to prevent DoS via unbounded allocation.
    // 1 billion elements * 8 bytes/Value = 8GB, well within heap limits.
    const MAX_ARRAY_LENGTH: usize = 1 << 30;
    if length > MAX_ARRAY_LENGTH {
        return Err(InterpreterError::Panic {
            message: format!("array length {} exceeds maximum {}", length, MAX_ARRAY_LENGTH),
        });
    }
    let init_val = state.get_reg(elem);

    let obj = state.heap.alloc_array(TypeId::UNIT, length)?;
    state.record_allocation();

    // Initialize all elements
    // SAFETY: `obj` was just returned from `state.heap.alloc_array()` with
    // `length` Value slots of storage. Skipping OBJECT_HEADER_SIZE lands on
    // the first Value slot of the allocation; the slice runs for `length`
    // 8-byte Value entries.
    let data_ptr = unsafe {
        (obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };
    for i in 0..length {
        // SAFETY: `i < length` and the allocation has `length` Value slots
        // reachable from `data_ptr`, so the write is in bounds and aligned.
        unsafe { *data_ptr.add(i) = init_val };
    }

    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
    Ok(DispatchResult::Continue)
}

/// GetF (0x62) - Get field: dst = obj.field
pub(in super::super) fn handle_get_field(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let obj = read_reg(state)?;
    let field_idx = read_varint(state)? as usize;

    let obj_val = state.get_reg(obj);

    // Handle CBGR register references: deref to get the actual pointer
    let obj_val = if is_cbgr_ref(&obj_val) {
        let (abs_index, _generation) = decode_cbgr_ref(obj_val.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        obj_val
    };

    // Handle SmallStr values: compiled stdlib Text methods (as_bytes, etc.) access
    // self.ptr (field 0), self.len (field 1), self.cap (field 2) via GetF.
    // SmallStr is the interpreter's inline NaN-boxed string representation (up to 6 bytes).
    // We need to materialize these fields for compiled code.
    if obj_val.is_small_string() {
        let ss = obj_val.as_small_string();
        let ss_bytes = ss.as_bytes();
        let len = ss_bytes.len();
        match field_idx {
            0 => {
                // ptr: allocate the bytes on the managed heap so they are tracked and freed.
                // Previous implementation used raw std::alloc::alloc() which leaked memory
                // on every SmallStr field access since the buffer was never freed.
                if len == 0 {
                    state.set_reg(dst, Value::from_ptr(std::ptr::null_mut::<u8>()));
                } else {
                    let bytes_copy = ss_bytes.to_vec();
                    let obj = state.heap.alloc_with_init(TypeId::UNIT, len, |data| {
                        data.copy_from_slice(&bytes_copy);
                    })?;
                    // Return pointer to data area (after ObjectHeader)
                    // SAFETY: `obj` was just allocated with `len` bytes of
                    // data storage after the header. Adding
                    // OBJECT_HEADER_SIZE yields a pointer to the initialized
                    // byte buffer.
                    let data_ptr = unsafe {
                        (obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE)
                    };
                    state.set_reg(dst, Value::from_ptr(data_ptr));
                }
            }
            1 => {
                // len
                state.set_reg(dst, Value::from_i64(len as i64));
            }
            2 => {
                // cap (same as len for SmallStr)
                state.set_reg(dst, Value::from_i64(len as i64));
            }
            _ => {
                state.set_reg(dst, Value::nil());
            }
        }
        return Ok(DispatchResult::Continue);
    }

    if !obj_val.is_ptr() || obj_val.is_nil() {
        return Err(InterpreterError::NullPointer);
    }

    let mut ptr = obj_val.as_ptr::<u8>();
    if ptr.is_null() {
        return Err(InterpreterError::NullPointer);
    }

    // Check for CBGR AllocationHeader raw field access.
    // When a pointer points to a tracked CBGR allocation base, fields are
    // packed u32 values (AllocationHeader: generation, size, flags, reserved)
    // with no ObjectHeader prefix.
    if state.cbgr_allocations.contains(&(ptr as usize)) {
        // AllocationHeader is 32 bytes: [size:4][align:4][generation:4][epoch:2][caps:2][type_id:4][flags:4][reserved:8]
        // Validate field_byte_offset to prevent out-of-bounds read on malicious bytecode.
        const CBGR_ALLOC_HEADER_SIZE: usize = 32;
        let field_byte_offset = field_idx.checked_mul(4).ok_or_else(|| {
            InterpreterError::Panic { message: "field index overflow in CBGR header access".into() }
        })?;
        if field_byte_offset + 4 > CBGR_ALLOC_HEADER_SIZE {
            return Err(InterpreterError::Panic {
                message: format!(
                    "field access out of bounds: offset {} + 4 > allocation header size {}",
                    field_byte_offset, CBGR_ALLOC_HEADER_SIZE
                ),
            });
        }
        // SAFETY: field_byte_offset is bounds-checked against CBGR_ALLOC_HEADER_SIZE above,
        // and ptr is verified to be a tracked CBGR allocation base.
        let u32_val = unsafe { *(ptr.add(field_byte_offset) as *const u32) };
        state.set_reg(dst, Value::from_i64(u32_val as i64));
        return Ok(DispatchResult::Continue);
    }

    // Auto-deref for CBGR Heap allocations (Heap.new(value)):
    // CBGR allocations are [AllocationHeader(32)][Value(8)].
    // The returned pointer is to the data area (after the 32-byte header).
    // If the stored Value is a pointer to a struct, follow it for field access.
    {
        let header_addr = (ptr as usize).wrapping_sub(32);
        if state.cbgr_allocations.contains(&header_addr) {
            // This pointer is the data area of a CBGR allocation.
            // Read the stored Value.
            // SAFETY: `ptr` points to the data area of a CBGR allocation
            // (verified via `header_addr` lookup in `cbgr_allocations`). The
            // data area begins with a single Value slot (Heap<T> layout),
            // which is aligned and initialized at allocation time.
            let inner_value = unsafe { *(ptr as *const Value) };
            if inner_value.is_ptr() && !inner_value.is_nil() {
                // The inner value is a pointer (e.g., to a struct). Follow it.
                ptr = inner_value.as_ptr::<u8>();
                if ptr.is_null() {
                    return Err(InterpreterError::NullPointer);
                }
            }
            // If inner_value is not a pointer (e.g., Int, Float), fall through
            // to read field at offset (which will handle it as a regular object).
        }
    }

    // Auto-deref for Heap<T> (variant wrapper with type_id >= 0x8000):
    // Heap objects are stored as variant wrappers where payload[0] is the inner value.
    // When accessing a field on a Heap<T>, we need to unwrap to get the inner T first.
    // SAFETY: Validate pointer alignment before casting to ObjectHeader (requires 4-byte alignment).
    if !(ptr as usize).is_multiple_of(std::mem::align_of::<heap::ObjectHeader>()) {
        return Err(InterpreterError::Panic {
            message: format!(
                "misaligned pointer {:p} for ObjectHeader (requires {}-byte alignment)",
                ptr,
                std::mem::align_of::<heap::ObjectHeader>()
            ),
        });
    }
    // SAFETY: Pointer alignment was verified immediately above, and `ptr`
    // is non-null. All VBC heap objects start with an ObjectHeader, so the
    // cast is layout-compatible. The reference is short-lived.
    let header = unsafe { &*(ptr as *const heap::ObjectHeader) };
    if header.type_id.0 >= 0x8000 {
        // This is a variant (e.g., Heap wrapper). Extract payload[0] as the inner object.
        let payload_offset = heap::OBJECT_HEADER_SIZE + 8; // skip tag + padding
        // SAFETY: Variant objects are laid out as [ObjectHeader, tag:u64,
        // payload...]; payload[0] sits at `OBJECT_HEADER_SIZE + 8` and is
        // initialized at construction. Alignment is satisfied (8 bytes).
        let inner_value = unsafe {
            *(ptr.add(payload_offset) as *const Value)
        };
        // The inner value should be a pointer to the actual record object
        if inner_value.is_ptr() && !inner_value.is_nil() {
            ptr = inner_value.as_ptr::<u8>();
            if ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }
        }
    }

    // Read field at offset
    let field_offset = field_idx.checked_mul(std::mem::size_of::<Value>())
        .ok_or_else(|| InterpreterError::Panic {
            message: "field offset overflow".into(),
        })?;
    // SAFETY: `field_offset` was produced by a checked multiplication, so
    // it cannot wrap. `ptr` is a live heap object whose data area holds
    // `header.size` bytes of Value slots. The emitter guarantees
    // `field_idx` is within the object's declared field count, so the
    // resulting pointer lies within the allocation.
    let data_ptr = unsafe {
        ptr.add(heap::OBJECT_HEADER_SIZE + field_offset) as *const Value
    };
    // SAFETY: `data_ptr` is 8-byte aligned (Value's alignment) and points
    // to an initialized Value slot (all fields are initialized at object
    // construction time).
    let value = unsafe { *data_ptr };
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

/// SetF (0x63) - Set field: obj.field = val
pub(in super::super) fn handle_set_field(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let obj = read_reg(state)?;
    let field_idx = read_varint(state)? as usize;
    let val = read_reg(state)?;

    let obj_val = state.get_reg(obj);

    // Handle CBGR register references: deref to get the actual pointer
    // This is essential when setting a field on a value accessed through a mutable reference:
    // e.g., `ref.field = value` where `ref: &mut Struct`
    let obj_val = if is_cbgr_ref(&obj_val) {
        let (abs_index, _generation) = decode_cbgr_ref(obj_val.as_i64());
        state.registers.get_absolute(abs_index)
    } else {
        obj_val
    };

    if !obj_val.is_ptr() || obj_val.is_nil() {
        return Err(InterpreterError::NullPointer);
    }

    let mut ptr = obj_val.as_ptr::<u8>();
    if ptr.is_null() {
        return Err(InterpreterError::NullPointer);
    }

    // Auto-deref for CBGR Heap allocations (Heap.new(value)):
    // Same logic as GetF - follow pointer through CBGR data area to inner struct.
    {
        let header_addr = (ptr as usize).wrapping_sub(32);
        if state.cbgr_allocations.contains(&header_addr) {
            // SAFETY: See handle_get_field CBGR auto-deref — `ptr` is the
            // data area of a CBGR allocation whose first slot is a Value.
            let inner_value = unsafe { *(ptr as *const Value) };
            if inner_value.is_ptr() && !inner_value.is_nil() {
                ptr = inner_value.as_ptr::<u8>();
                if ptr.is_null() {
                    return Err(InterpreterError::NullPointer);
                }
            }
        }
    }

    // Auto-deref for Heap<T> (variant wrapper with type_id >= 0x8000)
    // SAFETY: Validate pointer alignment before casting to ObjectHeader (requires 4-byte alignment).
    if !(ptr as usize).is_multiple_of(std::mem::align_of::<heap::ObjectHeader>()) {
        return Err(InterpreterError::Panic {
            message: format!(
                "misaligned pointer {:p} for ObjectHeader (requires {}-byte alignment)",
                ptr,
                std::mem::align_of::<heap::ObjectHeader>()
            ),
        });
    }
    // SAFETY: See handle_get_field — alignment verified above, `ptr` is a
    // live heap object, and all objects begin with ObjectHeader.
    let header = unsafe { &*(ptr as *const heap::ObjectHeader) };
    if header.type_id.0 >= 0x8000 {
        let payload_offset = heap::OBJECT_HEADER_SIZE + 8;
        // SAFETY: See handle_get_field — variant payload layout applies.
        let inner_value = unsafe {
            *(ptr.add(payload_offset) as *const Value)
        };
        if inner_value.is_ptr() && !inner_value.is_nil() {
            ptr = inner_value.as_ptr::<u8>();
            if ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }
        }
    }

    let value = state.get_reg(val);
    let field_offset = field_idx.checked_mul(std::mem::size_of::<Value>())
        .ok_or_else(|| InterpreterError::Panic {
            message: "field offset overflow".into(),
        })?;
    let data_ptr = unsafe {
        ptr.add(heap::OBJECT_HEADER_SIZE + field_offset) as *mut Value
    };
    unsafe { *data_ptr = value };
    Ok(DispatchResult::Continue)
}

/// GetE (0x64) - Get element: dst = arr[idx]
pub(in super::super) fn handle_get_index(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let arr = read_reg(state)?;
    let idx = read_reg(state)?;

    let arr_val = state.get_reg(arr);

    // Handle FatRef (slice) indexing first - FatRef also passes is_ptr() so check this before
    if arr_val.is_fat_ref() {
        let fat_ref = arr_val.as_fat_ref();
        let raw_idx_val = state.get_reg(idx);
        let idx_val = raw_idx_val.as_i64() as usize;
        let len = fat_ref.len() as usize;

        // Bounds check
        if idx_val >= len {
            return Err(InterpreterError::Panic {
                message: format!("Slice index out of bounds: index {} but length is {}", idx_val, len),
            });
        }

        // Get the element from the slice's backing array
        // The FatRef ptr points to the start of the slice data
        let base_ptr = fat_ref.ptr();
        if base_ptr.is_null() {
            return Err(InterpreterError::NullPointer);
        }

        // Check elem_size stored in reserved field:
        // 0 = NaN-boxed Value (8 bytes), 1/2/4/8 = raw integer size
        let elem_size = fat_ref.reserved;
        // eprintln!("[DEBUG FatRef GetE] elem_size={}, idx={}", elem_size, idx_val);

        let element = match elem_size {
            0 => {
                // NaN-boxed Values (LIST, generic arrays)
                let element_ptr = unsafe { (base_ptr as *const Value).add(idx_val) };
                unsafe { *element_ptr }
            }
            1 => {
                // Raw bytes (u8)
                let element_ptr = unsafe { base_ptr.add(idx_val) };
                Value::from_i64(unsafe { *element_ptr } as i64)
            }
            2 => {
                // Raw i16
                let element_ptr = unsafe { base_ptr.add(idx_val * 2) as *const i16 };
                Value::from_i64(unsafe { std::ptr::read_unaligned(element_ptr) } as i64)
            }
            4 => {
                // Raw i32
                let element_ptr = unsafe { base_ptr.add(idx_val * 4) as *const i32 };
                Value::from_i64(unsafe { std::ptr::read_unaligned(element_ptr) } as i64)
            }
            8 => {
                // Raw i64 (typed arrays like [Int; N])
                let element_ptr = unsafe { base_ptr.add(idx_val * 8) as *const i64 };
                Value::from_i64(unsafe { std::ptr::read_unaligned(element_ptr) })
            }
            _ => {
                // Unknown elem_size, fall back to Value
                let element_ptr = unsafe { (base_ptr as *const Value).add(idx_val) };
                unsafe { *element_ptr }
            }
        };
        // eprintln!("[DEBUG FatRef GetE] element={:?}", element);
        state.set_reg(dst, element);
        return Ok(DispatchResult::Continue);
    }

    if !arr_val.is_ptr() {
        // Transparent newtype access: .0 on a scalar value returns the value itself.
        // This handles `type Capabilities is (u32);` where `self.0` accesses the
        // underlying Int value through a type alias parsed as transparent.
        let idx_val = state.get_reg(idx);
        if idx_val.is_int() && idx_val.as_i64() == 0 {
            state.set_reg(dst, arr_val);
            return Ok(DispatchResult::Continue);
        }
        return Err(InterpreterError::InvalidOperand { message: format!("GetE: expected pointer in R{}, got tag={:?}", arr.0, arr_val.tag()) });
    }
    let ptr = arr_val.as_ptr::<u8>();
    if ptr.is_null() {
        return Err(InterpreterError::NullPointer);
    }

    // Check if this is a List (3 Values = len, cap, backing_ptr), byte array, or Value array
    let header = unsafe { &*(ptr as *const heap::ObjectHeader) };
    let header_size = header.size as usize;
    // eprintln!("[DEBUG GetE] ptr={:p}, type_id={:?}, size={}", ptr, header.type_id, header_size);

    // Check for Map first (index may not be an integer)
    if header.type_id == TypeId::MAP {
        // Map index read: map[key] → delegate to map lookup
        let key = state.get_reg(idx);
        let header_ptr = unsafe {
            ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
        };
        let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
        let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };
        let entries_data = unsafe {
            entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
        };
        let hash = value_hash(key);
        let mut map_idx = hash % capacity;
        let start_idx = map_idx;
        loop {
            let entry_key = unsafe { *entries_data.add(map_idx * 2) };
            if entry_key.is_unit() {
                state.set_reg(dst, Value::unit());
                break;
            }
            if value_eq(entry_key, key) {
                let entry_val = unsafe { *entries_data.add(map_idx * 2 + 1) };
                state.set_reg(dst, entry_val);
                break;
            }
            map_idx = (map_idx + 1) % capacity;
            if map_idx == start_idx {
                state.set_reg(dst, Value::unit());
                break;
            }
        }
    } else {
        // Check if index is a Range object (for slicing: arr[start..end])
        let idx_val = state.get_reg(idx);
        let is_range = idx_val.is_ptr() && {
            let idx_ptr = idx_val.as_ptr::<u8>();
            !idx_ptr.is_null() && {
                let idx_header = unsafe { &*(idx_ptr as *const heap::ObjectHeader) };
                idx_header.type_id == TypeId::RANGE
            }
        };

        if is_range {
            // Range indexing: arr[start..end] → create a new array with slice elements
            let idx_ptr = idx_val.as_ptr::<u8>();
            let range_data = unsafe {
                (idx_ptr as *const u8).add(heap::OBJECT_HEADER_SIZE) as *const Value
            };
            let start_val = unsafe { *range_data };
            let end_val = unsafe { *range_data.add(1) };
            let inclusive_val = unsafe { *range_data.add(2) };

            let start = start_val.as_i64() as usize;
            let mut end = end_val.as_i64() as usize;
            if inclusive_val.is_bool() && inclusive_val.as_bool() {
                end += 1;
            }

            // Determine element count of the source array
            let element_count = if header.type_id == TypeId::LIST {
                let data_ptr = unsafe {
                    ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                };
                (unsafe { (*data_ptr).as_i64() }) as usize
            } else if header.type_id == TypeId::U8 {
                header_size
            } else {
                header_size / std::mem::size_of::<Value>()
            };

            // Bounds check
            if start > element_count || end > element_count || start > end {
                return Err(InterpreterError::Panic {
                    message: format!("Index out of bounds: slice [{}..{}] exceeds length {}", start, end, element_count),
                });
            }

            let slice_len = end - start;

            if header.type_id == TypeId::U8 {
                let new_obj = state.heap.alloc(TypeId::U8, slice_len)?;
                state.record_allocation();
                let src = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE + start) };
                let dst_data = unsafe {
                    (new_obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE)
                };
                unsafe { std::ptr::copy_nonoverlapping(src, dst_data, slice_len); }
                state.set_reg(dst, Value::from_ptr(new_obj.as_ptr() as *mut u8));
            } else if header.type_id == TypeId::LIST {
                let data_ptr = unsafe {
                    ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                };
                let backing = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
                let new_obj = state.heap.alloc_array(TypeId::UNIT, slice_len)?;
                state.record_allocation();
                let dst_data = unsafe {
                    (new_obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
                };
                for i in 0..slice_len {
                    let elem = unsafe {
                        *((backing as *const u8).add(heap::OBJECT_HEADER_SIZE + (start + i) * std::mem::size_of::<Value>()) as *const Value)
                    };
                    unsafe { *dst_data.add(i) = elem; }
                }
                state.set_reg(dst, Value::from_ptr(new_obj.as_ptr() as *mut u8));
            } else {
                let new_obj = state.heap.alloc_array(TypeId::UNIT, slice_len)?;
                state.record_allocation();
                let src_data = unsafe {
                    ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
                };
                let dst_data = unsafe {
                    (new_obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
                };
                for i in 0..slice_len {
                    unsafe { *dst_data.add(i) = *src_data.add(start + i); }
                }
                state.set_reg(dst, Value::from_ptr(new_obj.as_ptr() as *mut u8));
            }
        } else {
        // Non-range: index must be an integer
        let index = idx_val.as_i64();

        if header.type_id == TypeId::U8 {
            // Byte array - elements are raw bytes, 1 byte each
            let element_count = header_size;

            if index < 0 || index as usize >= element_count {
                return Err(InterpreterError::IndexOutOfBounds {
                    index,
                    length: element_count,
                });
            }

            let data_ptr = unsafe {
                ptr.add(heap::OBJECT_HEADER_SIZE + index as usize)
            };
            let byte_value = unsafe { *data_ptr };
            state.set_reg(dst, Value::from_i64(byte_value as i64));
        } else if header.type_id == TypeId::U16 {
            // 16-bit typed array
            let element_count = header_size / 2;
            if index < 0 || index as usize >= element_count {
                return Err(InterpreterError::IndexOutOfBounds { index, length: element_count });
            }
            let data_ptr = unsafe {
                ptr.add(heap::OBJECT_HEADER_SIZE + (index as usize) * 2) as *const u16
            };
            state.set_reg(dst, Value::from_i64(unsafe { *data_ptr } as i64));
        } else if header.type_id == TypeId::U32 {
            // 32-bit typed array (Int32, UInt32, Float32)
            let element_count = header_size / 4;
            if index < 0 || index as usize >= element_count {
                return Err(InterpreterError::IndexOutOfBounds { index, length: element_count });
            }
            let data_ptr = unsafe {
                ptr.add(heap::OBJECT_HEADER_SIZE + (index as usize) * 4) as *const i32
            };
            state.set_reg(dst, Value::from_i64(unsafe { *data_ptr } as i64));
        } else if header.type_id == TypeId::U64 {
            // 64-bit typed array (Int64, UInt64, Float64)
            let element_count = header_size / 8;
            if index < 0 || index as usize >= element_count {
                return Err(InterpreterError::IndexOutOfBounds { index, length: element_count });
            }
            let data_ptr = unsafe {
                ptr.add(heap::OBJECT_HEADER_SIZE + (index as usize) * 8) as *const i64
            };
            state.set_reg(dst, Value::from_i64(unsafe { *data_ptr }));
        } else if header.type_id == TypeId::LIST {
            // List layout: [len: Value, cap: Value, backing_ptr: Value]
            let data_ptr = unsafe {
                ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
            };
            let len = unsafe { (*data_ptr).as_i64() } as usize;
            let backing = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() }; // backing_ptr is at index 2

            if index < 0 || index as usize >= len {
                return Err(InterpreterError::IndexOutOfBounds {
                    index,
                    length: len,
                });
            }

            let elem_offset = index as usize * std::mem::size_of::<Value>();
            let data_ptr = unsafe {
                backing.add(heap::OBJECT_HEADER_SIZE + elem_offset)
            };
            let value = unsafe { *(data_ptr as *const Value) };
            state.set_reg(dst, value);
        } else {
            // Array/Tuple - elements are stored directly in the object data
            let element_count = header_size / std::mem::size_of::<Value>();

            if index < 0 || index as usize >= element_count {
                return Err(InterpreterError::IndexOutOfBounds {
                    index,
                    length: element_count,
                });
            }

            let elem_offset = index as usize * std::mem::size_of::<Value>();
            let data_ptr = unsafe {
                ptr.add(heap::OBJECT_HEADER_SIZE + elem_offset)
            };
            let value = unsafe { *(data_ptr as *const Value) };
            state.set_reg(dst, value);
        }
        } // end non-range
    }
    Ok(DispatchResult::Continue)
}

/// SetE (0x65) - Set element: arr[idx] = val
pub(in super::super) fn handle_set_index(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let arr = read_reg(state)?;
    let idx = read_reg(state)?;
    let val = read_reg(state)?;

    let ptr = state.get_reg(arr).as_ptr::<u8>();
    if ptr.is_null() {
        return Err(InterpreterError::NullPointer);
    }

    let value = state.get_reg(val);

    // Check if this is a List (3 Values = len, cap, backing_ptr), byte array, or Value array
    let header = unsafe { &*(ptr as *const heap::ObjectHeader) };
    let header_size = header.size as usize;

    // Check for Map first (index may not be an integer)
    if header.type_id == TypeId::MAP {
        let key = state.get_reg(idx);
        // Reuse map set logic inline
        let header_ptr = unsafe {
            ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
        };
        let mut count = unsafe { (*header_ptr).as_i64() } as usize;
        let mut capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
        let mut entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };

        // Check load factor and grow if needed (> 70% full)
        if count * 10 >= capacity * 7 {
            let new_capacity = capacity * 2;
            let new_entries = state.heap.alloc_array(TypeId::UNIT, new_capacity * 2)?;
            state.record_allocation();
            let new_entries_ptr = new_entries.as_ptr() as *mut u8;
            let new_data = unsafe {
                new_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
            };
            for i in 0..(new_capacity * 2) {
                unsafe { *new_data.add(i) = Value::unit(); }
            }
            let old_data = unsafe {
                entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
            };
            for i in 0..capacity {
                let old_key = unsafe { *old_data.add(i * 2) };
                if !old_key.is_unit() {
                    let old_val = unsafe { *old_data.add(i * 2 + 1) };
                    let hash = value_hash(old_key);
                    let mut idx = hash % new_capacity;
                    loop {
                        let slot_key = unsafe { *new_data.add(idx * 2) };
                        if slot_key.is_unit() {
                            unsafe {
                                *new_data.add(idx * 2) = old_key;
                                *new_data.add(idx * 2 + 1) = old_val;
                            }
                            break;
                        }
                        idx = (idx + 1) % new_capacity;
                    }
                }
            }
            capacity = new_capacity;
            entries_ptr = new_entries_ptr;
            unsafe {
                *header_ptr.add(1) = Value::from_i64(new_capacity as i64);
                *header_ptr.add(2) = Value::from_ptr(new_entries_ptr);
            }
        }

        let entries_data = unsafe {
            entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
        };
        let hash = value_hash(key);
        let mut map_idx = hash % capacity;
        loop {
            let entry_key = unsafe { *entries_data.add(map_idx * 2) };
            if entry_key.is_unit() {
                // Empty slot — insert
                unsafe {
                    *entries_data.add(map_idx * 2) = key;
                    *entries_data.add(map_idx * 2 + 1) = value;
                }
                count += 1;
                unsafe { *header_ptr = Value::from_i64(count as i64); }
                break;
            }
            if value_eq(entry_key, key) {
                // Update existing
                unsafe { *entries_data.add(map_idx * 2 + 1) = value; }
                break;
            }
            map_idx = (map_idx + 1) % capacity;
        }
    } else {
        // Non-map: index must be an integer
        let index = state.get_reg(idx).as_i64();

        if header.type_id == TypeId::U8 {
            // Byte array - elements are raw bytes, 1 byte each
            let element_count = header_size;
            if index < 0 || index as usize >= element_count {
                return Err(InterpreterError::IndexOutOfBounds { index, length: element_count });
            }
            let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE + index as usize) };
            let byte_value = value.as_i64() as u8;
            unsafe { *data_ptr = byte_value };
        } else if header.type_id == TypeId::U16 {
            let element_count = header_size / 2;
            if index < 0 || index as usize >= element_count {
                return Err(InterpreterError::IndexOutOfBounds { index, length: element_count });
            }
            let data_ptr = unsafe {
                ptr.add(heap::OBJECT_HEADER_SIZE + (index as usize) * 2) as *mut u16
            };
            unsafe { *data_ptr = value.as_i64() as u16 };
        } else if header.type_id == TypeId::U32 {
            let element_count = header_size / 4;
            if index < 0 || index as usize >= element_count {
                return Err(InterpreterError::IndexOutOfBounds { index, length: element_count });
            }
            let data_ptr = unsafe {
                ptr.add(heap::OBJECT_HEADER_SIZE + (index as usize) * 4) as *mut i32
            };
            unsafe { *data_ptr = value.as_i64() as i32 };
        } else if header.type_id == TypeId::U64 {
            let element_count = header_size / 8;
            if index < 0 || index as usize >= element_count {
                return Err(InterpreterError::IndexOutOfBounds { index, length: element_count });
            }
            let data_ptr = unsafe {
                ptr.add(heap::OBJECT_HEADER_SIZE + (index as usize) * 8) as *mut i64
            };
            unsafe { *data_ptr = value.as_i64() };
        } else if header.type_id == TypeId::LIST {
            // List layout: [len: Value, cap: Value, backing_ptr: Value]
            let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
            let len = unsafe { (*data_ptr).as_i64() } as usize;
            let backing = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
            if index < 0 || index as usize >= len {
                return Err(InterpreterError::IndexOutOfBounds { index, length: len });
            }
            let elem_offset = index as usize * std::mem::size_of::<Value>();
            let data_ptr = unsafe { backing.add(heap::OBJECT_HEADER_SIZE + elem_offset) };
            unsafe { *(data_ptr as *mut Value) = value };
        } else {
            // Array/Tuple - elements are stored directly
            let element_count = header_size / std::mem::size_of::<Value>();
            if index < 0 || index as usize >= element_count {
                return Err(InterpreterError::IndexOutOfBounds { index, length: element_count });
            }
            let elem_offset = index as usize * std::mem::size_of::<Value>();
            let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE + elem_offset) };
            unsafe { *(data_ptr as *mut Value) = value };
        }
    }
    Ok(DispatchResult::Continue)
}

/// Len (0x66) - Get array/list length: dst = arr.len()
pub(in super::super) fn handle_array_len(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let arr = read_reg(state)?;
    // Read and discard type_hint byte (used by LLVM lowering, not interpreter).
    let _type_hint = read_u8(state)?;

    let mut val = state.get_reg(arr);

    // Handle CBGR register-based reference: decode and dereference
    if is_cbgr_ref(&val) {
        let (abs_index, _generation) = decode_cbgr_ref(val.as_i64());
        val = state.registers.get_absolute(abs_index);
    }

    // Handle ThinRef: dereference to get the actual value
    if val.is_thin_ref() {
        let thin_ref = val.as_thin_ref();
        if thin_ref.ptr.is_null() {
            return Err(InterpreterError::NullPointer);
        }
        // ThinRef points to a Value in memory (e.g., variant field)
        val = unsafe { *(thin_ref.ptr as *const Value) };
    }

    // Check if this pointer is to a CBGR-tracked variant field pointer (from ref binding).
    // These point to Value data, not ObjectHeaders. We need to dereference to get the actual value.
    if val.is_ptr() && !val.is_nil() {
        let ptr_addr = val.as_ptr::<u8>() as usize;
        if state.cbgr_mutable_ptrs.contains(&ptr_addr) {
            // This is a pointer to a Value (from GetVariantDataRef)
            val = unsafe { *(ptr_addr as *const Value) };
        }
    }

    // Handle small strings: return byte length directly from NaN-boxed value
    if val.is_small_string() {
        let ss = val.as_small_string();
        state.set_reg(dst, Value::from_i64(ss.len() as i64));
        return Ok(DispatchResult::Continue);
    }

    // Handle FatRef (slices): get length from metadata
    if val.is_fat_ref() {
        let fat_ref = val.as_fat_ref();
        state.set_reg(dst, Value::from_i64(fat_ref.len() as i64));
        return Ok(DispatchResult::Continue);
    }

    // Handle integer-encoded string IDs: look up in string table
    if val.is_int() {
        let str_id = val.as_i64() as u32;
        if let Some(s) = state.module.get_string(crate::types::StringId(str_id)) {
            state.set_reg(dst, Value::from_i64(s.len() as i64));
            return Ok(DispatchResult::Continue);
        }
    }

    let ptr = val.as_ptr::<u8>();
    if ptr.is_null() {
        return Err(InterpreterError::NullPointer);
    }

    let header = unsafe { &*(ptr as *const heap::ObjectHeader) };
    let header_size = header.size as usize;

    // Heuristic: If the type_id is very large or the size is unreasonable,
    // this might be a pointer to a Value (reference), not an ObjectHeader.
    // Try dereferencing as a Value and recursively getting its length.
    if header.type_id.0 > 0x8000 || header_size > 1_000_000_000 {
        // This looks like a reference to a Value, not an ObjectHeader
        let inner_val = unsafe { *(ptr as *const Value) };
        if inner_val.is_small_string() {
            let ss = inner_val.as_small_string();
            state.set_reg(dst, Value::from_i64(ss.len() as i64));
            return Ok(DispatchResult::Continue);
        } else if inner_val.is_ptr() && !inner_val.is_nil() {
            // Recursively get length from the inner value
            let inner_ptr = inner_val.as_ptr::<u8>();
            let inner_header = unsafe { &*(inner_ptr as *const heap::ObjectHeader) };
            if inner_header.type_id == crate::types::TypeId::TEXT || inner_header.type_id == crate::types::TypeId(0x0001) {
                // Heap string layout: [ObjectHeader][len: u64][bytes...]
                let len_ptr = unsafe { inner_ptr.add(heap::OBJECT_HEADER_SIZE) as *const u64 };
                let len = (unsafe { *len_ptr }) as i64;
                state.set_reg(dst, Value::from_i64(len));
                return Ok(DispatchResult::Continue);
            }
        }
    }

    let length = if header.type_id == TypeId::U8 {
        // Byte array - size is the length
        header_size
    } else if header.type_id == TypeId::U16 {
        header_size / 2
    } else if header.type_id == TypeId::U32 {
        header_size / 4
    } else if header.type_id == TypeId::U64 {
        header_size / 8
    } else if header.type_id == TypeId::LIST {
        // List layout: [len, cap, backing_ptr] - read len
        let data_ptr = unsafe {
            ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
        };
        let len = (unsafe { (*data_ptr).as_i64() }) as usize;
        // eprintln!("[DEBUG Len instruction] List ptr={:?}, len={}", ptr, len);
        len
    } else if header.type_id == TypeId::MAP || header.type_id == TypeId::SET {
        // Map/Set layout: [count, capacity, entries_ptr] - read count
        let data_ptr = unsafe {
            ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
        };
        (unsafe { (*data_ptr).as_i64() }) as usize
    } else if header.type_id == crate::types::TypeId::TEXT || header.type_id == crate::types::TypeId(0x0001) {
        // Heap string layout: [ObjectHeader][len: u64][bytes...]
        let len_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const u64 };
        (unsafe { *len_ptr }) as usize
    } else if header.type_id == TypeId::CHANNEL {
        // Channel layout: [len, cap, head, buffer_ptr, closed] — read len (field 0)
        let data_ptr = unsafe {
            ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
        };
        (unsafe { (*data_ptr).as_i64() }) as usize
    } else if header.type_id == TypeId::DEQUE {
        // Deque layout: [data, head, len, cap] — read len (field 2)
        let data_ptr = unsafe {
            ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
        };
        (unsafe { (*data_ptr.add(2)).as_i64() }) as usize
    } else {
        // Array/Tuple - elements stored directly, size / sizeof(Value)
        header_size / std::mem::size_of::<Value>()
    };

    state.set_reg(dst, Value::from_i64(length as i64));
    Ok(DispatchResult::Continue)
}

/// NewG (0x61) - Allocate generic type
/// Same as New but reads additional type parameter count.
/// In the interpreter, generic types are erased — all values are NaN-boxed —
/// so type parameters are read and discarded, and allocation proceeds like New.
pub(in super::super) fn handle_new_generic(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let type_id_val = read_varint(state)? as u32;
    let type_id = TypeId(type_id_val);
    let field_count = read_varint(state)? as usize;
    let type_param_count = read_varint(state)? as usize;

    // Skip type parameter registers (type erasure in interpreter)
    for _ in 0..type_param_count {
        let _type_param = read_reg(state)?;
    }

    // Allocate like a regular New
    let size = field_count.max(1) * std::mem::size_of::<Value>();
    let obj = state.heap.alloc(type_id, size)?;
    state.record_allocation();

    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
    Ok(DispatchResult::Continue)
}

/// NewList (0x68) - Create new list
pub(in super::super) fn handle_new_list(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;

    // List layout: [len: Value, cap: Value, backing_ptr: Value]
    let cap: usize = 16; // Default capacity

    let obj = state.heap.alloc(TypeId::LIST, 3 * std::mem::size_of::<Value>())?;
    state.record_allocation();
    // eprintln!("[DEBUG NewList] created obj at {:p}, TypeId::LIST={:?}", obj.as_ptr(), TypeId::LIST);
    let _verify_header = unsafe { &*(obj.as_ptr() as *const heap::ObjectHeader) };
    // eprintln!("[DEBUG NewList] verify header type_id={:?}", verify_header.type_id);

    let data_ptr = unsafe {
        (obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };

    // Allocate backing array
    let backing = state.heap.alloc_array(TypeId::LIST, cap)?;
    state.record_allocation();

    // Initialize list: len=0, cap, backing_ptr
    unsafe {
        *data_ptr = Value::from_i64(0);         // len
        *data_ptr.add(1) = Value::from_i64(cap as i64);  // cap
        *data_ptr.add(2) = Value::from_ptr(backing.as_ptr() as *mut u8);  // backing_ptr
    }

    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
    Ok(DispatchResult::Continue)
}

/// ListPush (0x69) - Push value to list
pub(in super::super) fn handle_list_push(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let list_reg = read_reg(state)?;
    let val_reg = read_reg(state)?;

    let ptr = state.get_reg(list_reg).as_ptr::<u8>();
    if ptr.is_null() {
        return Err(InterpreterError::NullPointer);
    }

    let value = state.get_reg(val_reg);

    // Read list header
    let data_ptr = unsafe {
        ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };
    let len = unsafe { (*data_ptr).as_i64() } as usize;
    let cap = unsafe { (*data_ptr.add(1)).as_i64() } as usize;
    let mut backing_ptr = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };

    if len >= cap {
        // Grow list: double capacity (minimum 16)
        let new_cap = if cap == 0 { 16 } else { cap * 2 };

        // Allocate new backing array
        let new_backing = state.heap.alloc_array(TypeId::UNIT, new_cap)?;
        state.record_allocation();
        let new_backing_ptr = new_backing.as_ptr() as *mut u8;

        // Copy existing elements
        if len > 0 {
            let old_data = unsafe {
                backing_ptr.add(heap::OBJECT_HEADER_SIZE)
            };
            let new_data = unsafe {
                new_backing_ptr.add(heap::OBJECT_HEADER_SIZE)
            };
            unsafe {
                std::ptr::copy_nonoverlapping(
                    old_data,
                    new_data,
                    len * std::mem::size_of::<Value>()
                );
            }
        }

        // Update list header with new capacity and backing pointer
        unsafe {
            *data_ptr.add(1) = Value::from_i64(new_cap as i64);
            *data_ptr.add(2) = Value::from_ptr(new_backing_ptr);
        }

        backing_ptr = new_backing_ptr;
    }

    // Write value to backing array
    let elem_ptr = unsafe {
        backing_ptr.add(heap::OBJECT_HEADER_SIZE + len * std::mem::size_of::<Value>()) as *mut Value
    };
    unsafe { *elem_ptr = value };

    // Update length
    unsafe { *data_ptr = Value::from_i64((len + 1) as i64) };

    Ok(DispatchResult::Continue)
}

/// ListPop (0x6A) - Pop value from list
pub(in super::super) fn handle_list_pop(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let list_reg = read_reg(state)?;

    let ptr = state.get_reg(list_reg).as_ptr::<u8>();
    if ptr.is_null() {
        return Err(InterpreterError::NullPointer);
    }

    // Read list header
    let data_ptr = unsafe {
        ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };
    let len = unsafe { (*data_ptr).as_i64() } as usize;

    if len == 0 {
        state.set_reg(dst, Value::unit());
        return Ok(DispatchResult::Continue);
    }

    let backing_ptr = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };

    // Read last element
    let elem_ptr = unsafe {
        backing_ptr.add(heap::OBJECT_HEADER_SIZE + (len - 1) * std::mem::size_of::<Value>()) as *const Value
    };
    let value = unsafe { *elem_ptr };

    // Update length
    unsafe { *data_ptr = Value::from_i64((len - 1) as i64) };

    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

/// Compute hash for a Value using FNV-1a on raw bits.
#[inline]
pub(crate) fn value_hash(v: Value) -> usize {
    let bits = v.to_bits();
    // FNV-1a 64-bit
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for byte in bits.to_le_bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash as usize
}

/// Check if two Values are equal for map key comparison.
#[inline]
pub(crate) fn value_eq(a: Value, b: Value) -> bool {
    // For map keys, we use bitwise equality (same as EqRef)
    a.to_bits() == b.to_bits()
}

/// NewMap (0x6B) - Create new map with default capacity.
///
/// Format: `NewMap dst`
/// Creates an empty map and stores pointer in dst.
pub(in super::super) fn handle_new_map(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let _capacity_hint = read_varint(state)?; // consume capacity varint from bytecode

    // Map header: [count, capacity, entries_ptr]
    const DEFAULT_CAP: usize = 16;

    let obj = state.heap.alloc(TypeId::MAP, 3 * std::mem::size_of::<Value>())?;
    state.record_allocation();

    let header_ptr = unsafe {
        (obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };

    // Allocate entries array: capacity * 2 Values (key, value pairs)
    let entries = state.heap.alloc_array(TypeId::MAP, DEFAULT_CAP * 2)?;
    state.record_allocation();
    let entries_ptr = entries.as_ptr() as *mut u8;

    // Initialize entries with unit (empty marker)
    let entries_data = unsafe {
        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };
    for i in 0..(DEFAULT_CAP * 2) {
        unsafe { *entries_data.add(i) = Value::unit(); }
    }

    // Initialize header: count=0, capacity, entries_ptr
    unsafe {
        *header_ptr = Value::from_i64(0);                           // count
        *header_ptr.add(1) = Value::from_i64(DEFAULT_CAP as i64);   // capacity
        *header_ptr.add(2) = Value::from_ptr(entries_ptr);          // entries_ptr
    }

    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
    Ok(DispatchResult::Continue)
}

/// MapGet (0x6C) - Get value from map.
///
/// Format: `MapGet dst, map, key`
/// Looks up key in map, stores value in dst (or unit if not found).
pub(in super::super) fn handle_map_get(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let map_reg = read_reg(state)?;
    let key_reg = read_reg(state)?;

    let map_ptr = state.get_reg(map_reg).as_ptr::<u8>();
    if map_ptr.is_null() {
        return Err(InterpreterError::NullPointer);
    }

    let key = state.get_reg(key_reg);

    // Read map header
    let header_ptr = unsafe {
        map_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
    };
    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };

    let entries_data = unsafe {
        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
    };

    // Linear probing lookup
    let hash = value_hash(key);
    let mut idx = hash % capacity;
    let start_idx = idx;

    loop {
        let entry_key = unsafe { *entries_data.add(idx * 2) };

        if entry_key.is_unit() {
            // Empty slot - key not found
            // Return 0 (matching AOT sentinel) for interpreter/AOT consistency
            state.set_reg(dst, Value::from_i64(0));
            return Ok(DispatchResult::Continue);
        }

        if value_eq(entry_key, key) {
            // Found key
            let entry_val = unsafe { *entries_data.add(idx * 2 + 1) };
            state.set_reg(dst, entry_val);
            return Ok(DispatchResult::Continue);
        }

        // Linear probe
        idx = (idx + 1) % capacity;
        if idx == start_idx {
            // Full circle - key not found (shouldn't happen with proper load factor)
            state.set_reg(dst, Value::from_i64(0));
            return Ok(DispatchResult::Continue);
        }
    }
}

/// MapSet (0x6D) - Set value in map.
///
/// Format: `MapSet map, key, val`
/// Sets map[key] = val, growing the map if necessary.
pub(in super::super) fn handle_map_set(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let map_reg = read_reg(state)?;
    let key_reg = read_reg(state)?;
    let val_reg = read_reg(state)?;

    let map_ptr = state.get_reg(map_reg).as_ptr::<u8>();
    if map_ptr.is_null() {
        return Err(InterpreterError::NullPointer);
    }

    let key = state.get_reg(key_reg);
    let val = state.get_reg(val_reg);

    // Read map header
    let header_ptr = unsafe {
        map_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };
    let mut count = unsafe { (*header_ptr).as_i64() } as usize;
    let mut capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
    let mut entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };

    // Check load factor and grow if needed (> 70% full)
    if count * 10 >= capacity * 7 {
        let new_capacity = capacity * 2;
        let new_entries = state.heap.alloc_array(TypeId::UNIT, new_capacity * 2)?;
        state.record_allocation();
        let new_entries_ptr = new_entries.as_ptr() as *mut u8;

        // Initialize new entries with unit
        let new_data = unsafe {
            new_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
        };
        for i in 0..(new_capacity * 2) {
            unsafe { *new_data.add(i) = Value::unit(); }
        }

        // Rehash existing entries
        let old_data = unsafe {
            entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
        };
        for i in 0..capacity {
            let old_key = unsafe { *old_data.add(i * 2) };
            if !old_key.is_unit() {
                let old_val = unsafe { *old_data.add(i * 2 + 1) };
                let hash = value_hash(old_key);
                let mut idx = hash % new_capacity;
                loop {
                    let slot_key = unsafe { *new_data.add(idx * 2) };
                    if slot_key.is_unit() {
                        unsafe {
                            *new_data.add(idx * 2) = old_key;
                            *new_data.add(idx * 2 + 1) = old_val;
                        }
                        break;
                    }
                    idx = (idx + 1) % new_capacity;
                }
            }
        }

        // Update header
        capacity = new_capacity;
        entries_ptr = new_entries_ptr;
        unsafe {
            *header_ptr.add(1) = Value::from_i64(new_capacity as i64);
            *header_ptr.add(2) = Value::from_ptr(new_entries_ptr);
        }
    }

    // Insert or update
    let entries_data = unsafe {
        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };

    let hash = value_hash(key);
    let mut idx = hash % capacity;

    loop {
        let entry_key = unsafe { *entries_data.add(idx * 2) };

        if entry_key.is_unit() {
            // Empty slot - insert new entry
            unsafe {
                *entries_data.add(idx * 2) = key;
                *entries_data.add(idx * 2 + 1) = val;
            }
            count += 1;
            unsafe { *header_ptr = Value::from_i64(count as i64); }
            return Ok(DispatchResult::Continue);
        }

        if value_eq(entry_key, key) {
            // Update existing entry
            unsafe {
                *entries_data.add(idx * 2 + 1) = val;
            }
            return Ok(DispatchResult::Continue);
        }

        // Linear probe
        idx = (idx + 1) % capacity;
    }
}

/// MapContains (0x6E) - Check if key exists in map.
///
/// Format: `MapContains dst, map, key`
/// Sets dst to true if key exists, false otherwise.
pub(in super::super) fn handle_map_contains(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let map_reg = read_reg(state)?;
    let key_reg = read_reg(state)?;

    let map_ptr = state.get_reg(map_reg).as_ptr::<u8>();
    if map_ptr.is_null() {
        return Err(InterpreterError::NullPointer);
    }

    let key = state.get_reg(key_reg);

    // Read map header
    let header_ptr = unsafe {
        map_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
    };
    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };

    let entries_data = unsafe {
        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
    };

    // Linear probing lookup
    let hash = value_hash(key);
    let mut idx = hash % capacity;
    let start_idx = idx;

    loop {
        let entry_key = unsafe { *entries_data.add(idx * 2) };

        if entry_key.is_unit() {
            // Empty slot - key not found
            state.set_reg(dst, Value::from_bool(false));
            return Ok(DispatchResult::Continue);
        }

        if value_eq(entry_key, key) {
            // Found key
            state.set_reg(dst, Value::from_bool(true));
            return Ok(DispatchResult::Continue);
        }

        // Linear probe
        idx = (idx + 1) % capacity;
        if idx == start_idx {
            // Full circle - key not found
            state.set_reg(dst, Value::from_bool(false));
            return Ok(DispatchResult::Continue);
        }
    }
}

/// Clone (0x78) - Clone a value (deep copy for heap objects).
pub(in super::super) fn handle_clone(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let value = state.get_reg(src);

    if value.is_ptr() && !value.is_nil() {
        let src_ptr = value.as_ptr::<u8>();
        if !src_ptr.is_null() {
            // Read the source object header to get type_id and size
            let header = unsafe { &*(src_ptr as *const heap::ObjectHeader) };
            let type_id = header.type_id;
            let data_size = header.size as usize;

            // Allocate a new object with same type and size
            let new_obj = state.heap.alloc(type_id, data_size)?;
            state.record_allocation();

            // Copy the data portion (not the header — alloc already set up a fresh header)
            let src_data = unsafe { src_ptr.add(heap::OBJECT_HEADER_SIZE) };
            let dst_data = new_obj.data_ptr();
            unsafe {
                std::ptr::copy_nonoverlapping(src_data, dst_data, data_size);
            }

            state.set_reg(dst, Value::from_ptr(new_obj.as_ptr() as *mut u8));
        } else {
            state.set_reg(dst, value);
        }
    } else {
        // Primitive value — just copy the NaN-boxed value
        state.set_reg(dst, value);
    }
    Ok(DispatchResult::Continue)
}

/// NewSet (0xC7) - Create new empty set.
///
/// Format: `NewSet dst`
/// Creates an empty set and stores pointer in dst.
pub(in super::super) fn handle_new_set(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let _capacity_hint = read_varint(state)?; // consume capacity varint from bytecode

    // Set header: [count, capacity, entries_ptr]
    const DEFAULT_CAP: usize = 16;

    let obj = state.heap.alloc(TypeId::SET, 3 * std::mem::size_of::<Value>())?;
    state.record_allocation();

    let header_ptr = unsafe {
        (obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };

    // Allocate entries array: capacity * 2 Values (key, dummy-value pairs)
    // Set uses the same [key, value] slot layout as Map to share the insert/contains
    // code paths. Each slot pair is 2 Values.
    let entries = state.heap.alloc_array(TypeId::UNIT, DEFAULT_CAP * 2)?;
    state.record_allocation();
    let entries_ptr = entries.as_ptr() as *mut u8;

    // Initialize entries with unit (empty marker)
    let entries_data = unsafe {
        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };
    for i in 0..(DEFAULT_CAP * 2) {
        unsafe { *entries_data.add(i) = Value::unit(); }
    }

    // Initialize header: count=0, capacity, entries_ptr
    unsafe {
        *header_ptr = Value::from_i64(0);                           // count
        *header_ptr.add(1) = Value::from_i64(DEFAULT_CAP as i64);   // capacity
        *header_ptr.add(2) = Value::from_ptr(entries_ptr);          // entries_ptr
    }

    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
    Ok(DispatchResult::Continue)
}

/// SetInsert (0xC8) - Insert element into set.
///
/// Format: `SetInsert set, elem`
/// Inserts elem into set if not already present.
pub(in super::super) fn handle_set_insert(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let set_reg = read_reg(state)?;
    let elem_reg = read_reg(state)?;

    let set_ptr = state.get_reg(set_reg).as_ptr::<u8>();
    if set_ptr.is_null() {
        return Err(InterpreterError::NullPointer);
    }

    let elem = state.get_reg(elem_reg);

    // Read set header
    let header_ptr = unsafe {
        set_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };
    let mut count = unsafe { (*header_ptr).as_i64() } as usize;
    let mut capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
    let mut entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };

    // Check load factor and grow if needed (> 70% full)
    if count * 10 >= capacity * 7 {
        let new_capacity = capacity * 2;
        let new_entries = state.heap.alloc_array(TypeId::UNIT, new_capacity)?;
        state.record_allocation();
        let new_entries_ptr = new_entries.as_ptr() as *mut u8;

        // Initialize new entries with unit
        let new_data = unsafe {
            new_entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
        };
        for i in 0..new_capacity {
            unsafe { *new_data.add(i) = Value::unit(); }
        }

        // Rehash existing entries
        let old_data = unsafe {
            entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
        };
        for i in 0..capacity {
            let old_elem = unsafe { *old_data.add(i) };
            if !old_elem.is_unit() {
                let hash = value_hash(old_elem);
                let mut idx = hash % new_capacity;
                loop {
                    let slot = unsafe { *new_data.add(idx) };
                    if slot.is_unit() {
                        unsafe { *new_data.add(idx) = old_elem; }
                        break;
                    }
                    idx = (idx + 1) % new_capacity;
                }
            }
        }

        // Update header
        capacity = new_capacity;
        entries_ptr = new_entries_ptr;
        unsafe {
            *header_ptr.add(1) = Value::from_i64(new_capacity as i64);
            *header_ptr.add(2) = Value::from_ptr(new_entries_ptr);
        }
    }

    // Insert element (if not already present)
    let entries_data = unsafe {
        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };

    let hash = value_hash(elem);
    let mut idx = hash % capacity;
    let start_idx = idx;

    loop {
        let slot = unsafe { *entries_data.add(idx) };

        if slot.is_unit() {
            // Empty slot - insert new element
            unsafe { *entries_data.add(idx) = elem; }
            count += 1;
            unsafe { *header_ptr = Value::from_i64(count as i64); }
            return Ok(DispatchResult::Continue);
        }

        if value_eq(slot, elem) {
            // Element already in set - no-op
            return Ok(DispatchResult::Continue);
        }

        // Linear probe
        idx = (idx + 1) % capacity;
        if idx == start_idx {
            // Full circle - this shouldn't happen with proper load factor
            return Err(InterpreterError::InvalidOperand { message: "Set is full".into() });
        }
    }
}

/// SetContains (0xC9) - Check if set contains element.
///
/// Format: `SetContains dst, set, elem`
/// Sets dst to true if elem is in set, false otherwise.
pub(in super::super) fn handle_set_contains(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let set_reg = read_reg(state)?;
    let elem_reg = read_reg(state)?;

    let set_ptr = state.get_reg(set_reg).as_ptr::<u8>();
    if set_ptr.is_null() {
        return Err(InterpreterError::NullPointer);
    }

    let elem = state.get_reg(elem_reg);

    // Read set header
    let header_ptr = unsafe {
        set_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
    };
    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };

    let entries_data = unsafe {
        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value
    };

    // Linear probing lookup
    let hash = value_hash(elem);
    let mut idx = hash % capacity;
    let start_idx = idx;

    loop {
        let slot = unsafe { *entries_data.add(idx) };

        if slot.is_unit() {
            // Empty slot - element not found
            state.set_reg(dst, Value::from_bool(false));
            return Ok(DispatchResult::Continue);
        }

        if value_eq(slot, elem) {
            // Found element
            state.set_reg(dst, Value::from_bool(true));
            return Ok(DispatchResult::Continue);
        }

        // Linear probe
        idx = (idx + 1) % capacity;
        if idx == start_idx {
            // Full circle - element not found
            state.set_reg(dst, Value::from_bool(false));
            return Ok(DispatchResult::Continue);
        }
    }
}

/// SetRemove (0xCA) - Remove element from set.
///
/// Format: `SetRemove set, elem`
/// Removes elem from set if present.
pub(in super::super) fn handle_set_remove(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let set_reg = read_reg(state)?;
    let elem_reg = read_reg(state)?;

    let set_ptr = state.get_reg(set_reg).as_ptr::<u8>();
    if set_ptr.is_null() {
        return Err(InterpreterError::NullPointer);
    }

    let elem = state.get_reg(elem_reg);

    // Read set header
    let header_ptr = unsafe {
        set_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };
    let mut count = unsafe { (*header_ptr).as_i64() } as usize;
    let capacity = unsafe { (*header_ptr.add(1)).as_i64() } as usize;
    let entries_ptr = unsafe { (*header_ptr.add(2)).as_ptr::<u8>() };

    let entries_data = unsafe {
        entries_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };

    // Find and remove element
    let hash = value_hash(elem);
    let mut idx = hash % capacity;
    let start_idx = idx;

    loop {
        let slot = unsafe { *entries_data.add(idx) };

        if slot.is_unit() {
            // Empty slot - element not found, nothing to remove
            return Ok(DispatchResult::Continue);
        }

        if value_eq(slot, elem) {
            // Found element - remove it using tombstone-free deletion
            // We need to rehash subsequent elements in the probe chain
            unsafe { *entries_data.add(idx) = Value::unit(); }
            count -= 1;
            unsafe { *header_ptr = Value::from_i64(count as i64); }

            // Rehash subsequent elements in probe chain
            let mut next_idx = (idx + 1) % capacity;
            while !unsafe { *entries_data.add(next_idx) }.is_unit() {
                let elem_to_rehash = unsafe { *entries_data.add(next_idx) };
                unsafe { *entries_data.add(next_idx) = Value::unit(); }

                // Reinsert the element
                let rehash = value_hash(elem_to_rehash);
                let mut new_idx = rehash % capacity;
                loop {
                    let new_slot = unsafe { *entries_data.add(new_idx) };
                    if new_slot.is_unit() {
                        unsafe { *entries_data.add(new_idx) = elem_to_rehash; }
                        break;
                    }
                    new_idx = (new_idx + 1) % capacity;
                }

                next_idx = (next_idx + 1) % capacity;
            }

            return Ok(DispatchResult::Continue);
        }

        // Linear probe
        idx = (idx + 1) % capacity;
        if idx == start_idx {
            // Full circle - element not found
            return Ok(DispatchResult::Continue);
        }
    }
}

/// NewDeque (0xCD) - Create new empty deque with default capacity.
///
/// Format: `NewDeque dst`
pub(in super::super) fn handle_new_deque(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let _capacity_hint = read_varint(state)?;

    const DEFAULT_CAP: usize = 16;

    // Deque header: [data(0), head(1), len(2), cap(3)]
    // Layout matches stdlib: type Deque<T> is { data, head, len, cap }
    let obj = state.heap.alloc(TypeId::DEQUE, 4 * std::mem::size_of::<Value>())?;
    state.record_allocation();

    let header_ptr = unsafe {
        (obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };

    let buffer = state.heap.alloc_array(TypeId::UNIT, DEFAULT_CAP)?;
    state.record_allocation();
    let buffer_ptr = buffer.as_ptr() as *mut u8;

    let buf_data = unsafe {
        buffer_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };
    for i in 0..DEFAULT_CAP {
        unsafe { *buf_data.add(i) = Value::unit(); }
    }

    unsafe {
        *header_ptr = Value::from_ptr(buffer_ptr);                 // data (index 0)
        *header_ptr.add(1) = Value::from_i64(0);                  // head (index 1)
        *header_ptr.add(2) = Value::from_i64(0);                  // len  (index 2)
        *header_ptr.add(3) = Value::from_i64(DEFAULT_CAP as i64); // cap  (index 3)
    }

    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
    Ok(DispatchResult::Continue)
}

/// NewChannel (0xDD) - Create new bounded channel.
///
/// Format: `NewChannel dst, capacity`
pub(in super::super) fn handle_new_channel(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let cap_reg = read_reg(state)?;
    let cap = state.get_reg(cap_reg).as_i64().max(1) as usize;

    // Channel header: [len, cap, head, buffer_ptr, closed]
    let obj = state.heap.alloc(TypeId::CHANNEL, 5 * std::mem::size_of::<Value>())?;
    state.record_allocation();

    let header_ptr = unsafe {
        (obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };

    let buffer = state.heap.alloc_array(TypeId::UNIT, cap)?;
    state.record_allocation();
    let buffer_ptr = buffer.as_ptr() as *mut u8;

    let buf_data = unsafe {
        buffer_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value
    };
    for i in 0..cap {
        unsafe { *buf_data.add(i) = Value::unit(); }
    }

    unsafe {
        *header_ptr = Value::from_i64(0);                  // len
        *header_ptr.add(1) = Value::from_i64(cap as i64);  // cap
        *header_ptr.add(2) = Value::from_i64(0);           // head
        *header_ptr.add(3) = Value::from_ptr(buffer_ptr);  // buffer_ptr
        *header_ptr.add(4) = Value::from_i64(0);           // closed (0=open)
    }

    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
    Ok(DispatchResult::Continue)
}

/// Push (0xCE) - Push value to argument stack.
///
/// Format: `Push src`
/// Pushes the value from src register onto the argument stack.
pub(in super::super) fn handle_push(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let src = read_reg(state)?;
    let value = state.get_reg(src);
    state.arg_stack.push(value);
    Ok(DispatchResult::Continue)
}

/// Pop (0xCF) - Pop value from argument stack.
///
/// Format: `Pop dst`
/// Pops a value from the argument stack into dst register.
pub(in super::super) fn handle_pop(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let value = state.arg_stack.pop().unwrap_or_default();
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

