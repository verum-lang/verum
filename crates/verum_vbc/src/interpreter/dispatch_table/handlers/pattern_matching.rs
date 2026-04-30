//! Pattern matching and variant instruction handlers for VBC interpreter.

use crate::instruction::Reg;
use crate::types::TypeId;
use crate::value::Value;
use super::super::super::error::InterpreterResult;
use super::super::super::state::InterpreterState;
use super::super::super::heap;
use super::super::DispatchResult;
use super::bytecode_io::*;
use super::cbgr_helpers::{is_cbgr_ref, decode_cbgr_ref};

// ============================================================================
// Pattern Matching + Variant Operations
// ============================================================================

/// AsVar (0x91) - Extract variant payload by field index.
///
/// Similar to GetVariantData but uses u8 for field index instead of varint.
pub(in super::super) fn handle_as_var(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let variant_reg = read_reg(state)?;
    let field_idx = read_varint(state)? as usize;

    let variant = state.get_reg(variant_reg);

    if variant.is_ptr() && !variant.is_nil() {
        let base_ptr = variant.as_ptr::<u8>();
        if !base_ptr.is_null() {
            // Payload starts at OBJECT_HEADER_SIZE + 8 (after tag + padding)
            let payload_offset = heap::OBJECT_HEADER_SIZE + 8;
            let field_offset = payload_offset + field_idx * std::mem::size_of::<Value>();
            unsafe {
                let field_ptr = base_ptr.add(field_offset) as *const Value;
                let value = std::ptr::read(field_ptr);
                state.set_reg(dst, value);
            }
        } else {
            state.set_reg(dst, Value::nil());
        }
    } else if variant.is_int() || variant.is_bool() || variant.is_float() {
        // Non-pointer value that is being treated as a variant payload.
        // This happens when compiled functions return a tagged value directly
        // (e.g., Some(42) might be stored as an inline value, not a heap variant).
        // In this case, the value itself IS the payload at field 0.
        if field_idx == 0 {
            state.set_reg(dst, variant);
        } else {
            state.set_reg(dst, Value::nil());
        }
    } else {
        state.set_reg(dst, Value::nil());
    }

    Ok(DispatchResult::Continue)
}

/// Switch (0x94) - Jump table dispatch.
pub(in super::super) fn handle_switch(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let selector = read_reg(state)?;
    let case_count = read_u8(state)? as usize;
    let default_offset = read_signed_varint(state)? as i32;


    let selector_val = state.get_reg(selector).as_i64() as usize;

    // Read all case offsets
    let mut case_offsets = Vec::with_capacity(case_count);
    for _ in 0..case_count {
        case_offsets.push(read_signed_varint(state)? as i32);
    }

    // Jump to appropriate case or default
    let offset = if selector_val < case_count {
        case_offsets[selector_val]
    } else {
        default_offset
    };

    let new_pc = (state.pc() as i64 + offset as i64) as u32;
    state.set_pc(new_pc);
    Ok(DispatchResult::Continue)
}

/// MatchGuard (0x95) - Evaluate match guard, jump if false.
pub(in super::super) fn handle_match_guard(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let cond = read_reg(state)?;
    let fail_offset = read_signed_varint(state)? as i32;

    if !state.get_reg(cond).is_truthy() {
        let new_pc = (state.pc() as i64 + fail_offset as i64) as u32;
        state.set_pc(new_pc);
    }
    Ok(DispatchResult::Continue)
}

// Generic/Type operation stubs
pub(in super::super) fn handle_specialize(_state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    // Instantiate (0x85) is a reserved opcode for runtime generic instantiation.
    // Verum uses compile-time monomorphization, so this opcode is never emitted.
    // Gracefully skip as a no-op if encountered in legacy/malformed bytecode.
    Ok(DispatchResult::Continue)
}

/// TypeOf - Return the runtime type tag of a value as an integer.
///
/// Encoding: opcode + dst:reg + src:reg
/// Effect: Inspects the NaN-boxed tag bits and stores a TypeId integer in `dst`.
pub(in super::super) fn handle_type_of(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let val = state.get_reg(src);

    let type_id: u32 = if val.is_float() {
        crate::types::TypeId::FLOAT.0
    } else if val.is_int() {
        crate::types::TypeId::INT.0
    } else if val.is_bool() {
        crate::types::TypeId::BOOL.0
    } else if val.is_unit() {
        crate::types::TypeId::UNIT.0
    } else if val.is_nil() {
        // nil maps to Unit
        crate::types::TypeId::UNIT.0
    } else if val.is_small_string() {
        crate::types::TypeId::TEXT.0
    } else if val.is_ptr() {
        crate::types::TypeId::PTR.0
    } else {
        // Fallback: treat as generic object/value
        0
    };

    state.set_reg(dst, Value::from_i64(type_id as i64));
    Ok(DispatchResult::Continue)
}

/// SizeOfG (0x83) - Get the size of a type in bytes.
///
/// Encoding: opcode + dst:reg + type_id:varint
/// Effect: Stores the size of the type (in bytes) into `dst`.
///
/// For builtin types, returns standard sizes:
/// - Unit/Bool: 1 byte
/// - Int/Float/Pointer: 8 bytes
/// - Value (NaN-boxed): 8 bytes
///
/// For user-defined types, looks up size from TypeDescriptor.
pub(in super::super) fn handle_size_of(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let type_id = crate::types::TypeId(read_varint(state)? as u32);

    // Get size based on type
    let size: i64 = if type_id.is_builtin() {
        // Builtin type sizes (INT=I64=2, FLOAT=F64=3 are aliases with same TypeId)
        match type_id {
            crate::types::TypeId::UNIT => 0,
            crate::types::TypeId::BOOL => 1,
            crate::types::TypeId::INT => 8,   // Also covers I64 (same TypeId)
            crate::types::TypeId::FLOAT => 8, // Also covers F64 (same TypeId)
            crate::types::TypeId::TEXT => std::mem::size_of::<Value>() as i64, // Text is a fat pointer
            crate::types::TypeId::U8 | crate::types::TypeId::I8 => 1,
            crate::types::TypeId::U16 | crate::types::TypeId::I16 => 2,
            crate::types::TypeId::U32 | crate::types::TypeId::I32 | crate::types::TypeId::F32 => 4,
            crate::types::TypeId::U64 | crate::types::TypeId::PTR => 8,
            _ => std::mem::size_of::<Value>() as i64, // Default to Value size
        }
    } else {
        // User-defined type - look up in type table
        match state.module.get_type(type_id) {
            Some(desc) => desc.size as i64,
            None => std::mem::size_of::<Value>() as i64, // Fallback to Value size
        }
    };

    state.set_reg(dst, Value::from_i64(size));
    Ok(DispatchResult::Continue)
}

/// AlignOfG (0x84) - Get the alignment of a type in bytes.
///
/// Encoding: opcode + dst:reg + type_id:varint
/// Effect: Stores the alignment of the type (in bytes) into `dst`.
///
/// For builtin types, returns standard alignments:
/// - Unit/Bool: 1 byte
/// - Int/Float/Pointer: 8 bytes
///
/// For user-defined types, looks up alignment from TypeDescriptor.
pub(in super::super) fn handle_align_of(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let type_id = crate::types::TypeId(read_varint(state)? as u32);

    // Get alignment based on type (INT=I64, FLOAT=F64 are aliases with same TypeId)
    let alignment: i64 = if type_id.is_builtin() {
        // Builtin type alignments
        match type_id {
            crate::types::TypeId::UNIT => 1,
            crate::types::TypeId::BOOL | crate::types::TypeId::U8 | crate::types::TypeId::I8 => 1,
            crate::types::TypeId::U16 | crate::types::TypeId::I16 => 2,
            crate::types::TypeId::U32 | crate::types::TypeId::I32 | crate::types::TypeId::F32 => 4,
            crate::types::TypeId::INT => 8,   // Also covers I64 (same TypeId)
            crate::types::TypeId::FLOAT => 8, // Also covers F64 (same TypeId)
            crate::types::TypeId::U64 | crate::types::TypeId::PTR => 8,
            _ => std::mem::align_of::<Value>() as i64, // Default to Value alignment
        }
    } else {
        // User-defined type - look up in type table
        match state.module.get_type(type_id) {
            Some(desc) => desc.alignment as i64,
            None => std::mem::align_of::<Value>() as i64, // Fallback to Value alignment
        }
    };

    state.set_reg(dst, Value::from_i64(alignment));
    Ok(DispatchResult::Continue)
}

/// MakeVariant (0x86) - Create a new variant with the specified tag.
///
/// Encoding: opcode + dst:reg + tag:varint
/// Effect: Allocates a new variant object with the given tag and stores pointer in `dst`.
///
/// Variant layout: ObjectHeader + [tag:u32][padding:u32][payload space...]
pub(in super::super) fn handle_make_variant(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let tag = read_varint(state)? as u32;
    let field_count = read_varint(state)? as u32;
    alloc_variant_into(state, dst, tag, field_count)?;
    Ok(DispatchResult::Continue)
}

/// Shared variant-allocation helper — used by both the legacy
/// `MakeVariant` (`0x86`) handler above and the typed
/// `MakeVariantTyped` (Extended sub-op `0x01`) handler in
/// `handlers/extended.rs`.  Performance-critical: a single
/// `heap.alloc_with_init` call with the in-place tag + field_count
/// store inside the closure (no per-instruction branching beyond
/// what was already in `MakeVariant`).
///
/// Centralising this in one helper guarantees that switching a
/// variant-construction site from `MakeVariant` to `MakeVariantTyped`
/// (Phase 3c) produces bit-equivalent runtime state — the
/// observable layout is determined entirely by `(tag, field_count)`.
#[inline]
pub(in super::super) fn alloc_variant_into(
    state: &mut InterpreterState,
    dst: Reg,
    tag: u32,
    field_count: u32,
) -> InterpreterResult<()> {
    // Legacy `MakeVariant` path — no parent-type info available, so
    // we fall back to the synthetic `0x8000+tag` sentinel that
    // downstream consumers (`format_variant_for_print_depth`,
    // pattern-match dispatch) recognise as "tag is meaningful but
    // type_id is not".  Prefer `alloc_variant_into_with_type_id`
    // when the codegen knows the parent sum-type id.
    alloc_variant_into_with_type_id(state, dst, tag, field_count, TypeId(0x8000 + tag))
}

/// Same as `alloc_variant_into` but the caller supplies the concrete
/// parent sum-type id (typically resolved by codegen via
/// `type_name_to_id.get(parent_type_name)`).  Storing the real type_id
/// in the heap header lets `format_variant_for_print_depth` resolve
/// the variant constructor name in O(N_variants_of_type) instead of
/// scanning every type in the module — and crucially produces the
/// correct name when distinct sum types share variant tags (e.g.
/// `Result.Err` and `ShellError.SpawnFailed` both have tag=1).
#[inline]
pub(in super::super) fn alloc_variant_into_with_type_id(
    state: &mut InterpreterState,
    dst: Reg,
    tag: u32,
    field_count: u32,
    type_id: TypeId,
) -> InterpreterResult<()> {
    let data_size = 8 + (field_count as usize) * std::mem::size_of::<Value>();
    let obj = state.heap.alloc_with_init(
        type_id,
        data_size,
        |data| {
            let tag_ptr = data.as_mut_ptr() as *mut u32;
            unsafe {
                *tag_ptr = tag;
                *tag_ptr.add(1) = field_count;
            }
        },
    )?;
    state.record_allocation();

    state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
    Ok(())
}

/// SetVariantData (0x87) - Set a field in a variant's payload.
///
/// Encoding: opcode + variant:reg + field:varint + value:reg
/// Effect: Stores `value` at the specified `field` offset in the variant's payload.
pub(in super::super) fn handle_set_variant_data(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let variant_reg = read_reg(state)?;
    let field = read_varint(state)? as usize;
    let value_reg = read_reg(state)?;

    let variant = state.get_reg(variant_reg);
    let value = state.get_reg(value_reg);

    if variant.is_ptr() && !variant.is_nil() {
        let base_ptr = variant.as_ptr::<u8>();
        if !base_ptr.is_null() {
            // Payload starts at OBJECT_HEADER_SIZE + 8 (after tag + padding)
            // Field offset is measured in Value-sized units
            let payload_offset = heap::OBJECT_HEADER_SIZE + 8;
            let field_offset = payload_offset + field * std::mem::size_of::<Value>();
            unsafe {
                let field_ptr = base_ptr.add(field_offset) as *mut Value;
                std::ptr::write(field_ptr, value);
            }
        }
    }

    Ok(DispatchResult::Continue)
}

/// GetVariantData (0x88) - Get a field from a variant's payload.
///
/// Encoding: opcode + dst:reg + variant:reg + field:varint
/// Effect: Reads the value at the specified `field` offset from the variant's payload into `dst`.
pub(in super::super) fn handle_get_variant_data(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let variant_reg = read_reg(state)?;
    let field = read_varint(state)? as usize;

    let mut variant = state.get_reg(variant_reg);


    // Auto-deref through register-based references (CBGR encoding)
    let mut deref_depth = 0;
    while is_cbgr_ref(&variant) && deref_depth < 8 {
        let (abs_index, _generation) = decode_cbgr_ref(variant.as_i64());
        let dereffed = state.registers.get_absolute(abs_index);
        if dereffed.to_bits() == variant.to_bits() {
            break;
        }
        variant = dereffed;
        deref_depth += 1;
    }

    if variant.is_ptr() && !variant.is_nil() {
        let base_ptr = variant.as_ptr::<u8>();
        if !base_ptr.is_null() {
            let payload_offset = heap::OBJECT_HEADER_SIZE + 8;
            let field_offset = payload_offset + field * std::mem::size_of::<Value>();
            unsafe {
                let field_ptr = base_ptr.add(field_offset) as *const Value;
                let value = std::ptr::read(field_ptr);
                state.set_reg(dst, value);
            }
        } else {
            state.set_reg(dst, Value::nil());
        }
    } else {
        state.set_reg(dst, Value::nil());
    }

    Ok(DispatchResult::Continue)
}

/// GetVariantDataRef (0x8B) - Get pointer to variant data field.
///
/// Unlike GetVariantData which copies the field value, this returns a pointer
/// to the field location within the variant. Used for `ref` and `ref mut`
/// pattern bindings to enable mutation through references.
///
/// Encoding: opcode + dst:reg + variant:reg + field:varint
/// Effect: Sets `dst` to a pointer to the field at `field` offset in the variant's payload.
pub(in super::super) fn handle_get_variant_data_ref(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let variant_reg = read_reg(state)?;
    let field = read_varint(state)? as usize;

    let mut variant = state.get_reg(variant_reg);

    // Auto-deref through register-based references (CBGR encoding)
    let mut deref_depth = 0;
    while is_cbgr_ref(&variant) && deref_depth < 8 {
        let (abs_index, _generation) = decode_cbgr_ref(variant.as_i64());
        let dereffed = state.registers.get_absolute(abs_index);
        if dereffed.to_bits() == variant.to_bits() {
            break;
        }
        variant = dereffed;
        deref_depth += 1;
    }

    if variant.is_ptr() && !variant.is_nil() {
        let base_ptr = variant.as_ptr::<u8>();
        if !base_ptr.is_null() {
            // Payload starts at OBJECT_HEADER_SIZE + 8 (after tag + padding)
            // Field offset is measured in Value-sized units
            let payload_offset = heap::OBJECT_HEADER_SIZE + 8;
            let field_offset = payload_offset + field * std::mem::size_of::<Value>();
            // Return pointer to the field (not the field value)
            let field_ptr = unsafe { base_ptr.add(field_offset) };
            let field_ptr_addr = field_ptr as usize;
            // Track this as a mutable pointer so Deref knows to read the Value from memory
            // (instead of doing identity deref like for regular heap objects)
            state.cbgr_mutable_ptrs.insert(field_ptr_addr);
            state.set_reg(dst, Value::from_ptr(field_ptr));
        } else {
            state.set_reg(dst, Value::nil());
        }
    } else {
        state.set_reg(dst, Value::nil());
    }

    Ok(DispatchResult::Continue)
}

/// IsVar (0x90) - Check if variant has a specific tag.
///
/// Encoding: opcode + dst:reg + value:reg + tag:varint
/// Effect: Sets `dst` to `true` if `value` has the specified `tag`, `false` otherwise.
///
/// Variant layout: ObjectHeader + [tag:u32][padding:u32][payload...]
/// - Tag is stored as u32 at offset OBJECT_HEADER_SIZE
pub(in super::super) fn handle_match_tag(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let value_reg = read_reg(state)?;
    let expected_tag = read_varint(state)? as u32;

    let mut value = state.get_reg(value_reg);

    // Auto-deref through register-based references (CBGR encoding)
    let mut deref_depth = 0;
    while is_cbgr_ref(&value) && deref_depth < 8 {
        let (abs_index, _generation) = decode_cbgr_ref(value.as_i64());
        let dereffed = state.registers.get_absolute(abs_index);
        if dereffed.to_bits() == value.to_bits() {
            break;
        }
        value = dereffed;
        deref_depth += 1;
    }

    // Check if value is a pointer to a variant object
    let matches = if value.is_ptr() && !value.is_nil() {
        let base_ptr = value.as_ptr::<u8>();
        if !base_ptr.is_null() {
            unsafe {
                let tag_ptr = base_ptr.add(heap::OBJECT_HEADER_SIZE) as *const u32;
                let actual_tag = *tag_ptr;
                actual_tag == expected_tag
            }
        } else {
            false
        }
    } else {
        // Not a pointer - can't be a variant
        false
    };

    state.set_reg(dst, Value::from_bool(matches));
    Ok(DispatchResult::Continue)
}

/// AsVar (0x91) - Extract variant payload into register.
///
/// Encoding: opcode + dst:reg + value:reg + tag:varint
/// Effect: Extracts the payload of `value` if it has the specified `tag`.
///
/// Variant layout: ObjectHeader + [tag:u32][padding:u32][payload:Value]
/// - Payload is stored at offset OBJECT_HEADER_SIZE + 8
pub(in super::super) fn handle_get_tag(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let value_reg = read_reg(state)?;

    let value = state.get_reg(value_reg);

    // Extract tag from variant object
    // Variant layout: ObjectHeader + [tag:u32][padding:u32][payload:Value]
    if value.is_ptr() && !value.is_nil() {
        let base_ptr = value.as_ptr::<u8>();
        if !base_ptr.is_null() {
            unsafe {
                let tag_offset = heap::OBJECT_HEADER_SIZE;
                let tag_ptr = base_ptr.add(tag_offset) as *const u32;
                let tag = std::ptr::read(tag_ptr);
                state.set_reg(dst, Value::from_i64(tag as i64));
            }
        } else {
            state.set_reg(dst, Value::from_i64(0));
        }
    } else {
        // Non-pointer values have tag 0 (default)
        state.set_reg(dst, Value::from_i64(0));
    }

    Ok(DispatchResult::Continue)
}

/// Unpack (0x92) - Unpack tuple into consecutive registers.
///
/// Encoding: opcode + dst_start + tuple + count
/// Effect: Unpacks `count` elements from tuple into registers starting at `dst_start`.
///
/// Tuple layout: ObjectHeader + [Value; count]
/// - Object header size is OBJECT_HEADER_SIZE bytes
/// - Each element is sizeof(Value) bytes
pub(in super::super) fn handle_unpack(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst_start = read_reg(state)?;
    let tuple_reg = read_reg(state)?;
    let count = read_u8(state)?;

    let tuple_value = state.get_reg(tuple_reg);

    // Tuples in VBC are heap-allocated objects with layout:
    // [ObjectHeader][Value0][Value1]...[ValueN]
    if tuple_value.is_ptr() && !tuple_value.is_nil() {
        let base_ptr = tuple_value.as_ptr::<u8>();
        if !base_ptr.is_null() {
            // Skip object header to get to the data
            let data_offset = heap::OBJECT_HEADER_SIZE;

            for i in 0..count as usize {
                // Calculate offset for each element
                let element_offset = data_offset + i * std::mem::size_of::<Value>();
                unsafe {
                    let element_ptr = base_ptr.add(element_offset) as *const Value;
                    let element = std::ptr::read(element_ptr);
                    state.set_reg(Reg(dst_start.0 + i as u16), element);
                }
            }
            return Ok(DispatchResult::Continue);
        }
    }

    // Fallback for non-pointer tuples (shouldn't normally happen)
    // Just copy the value to the first destination register
    if count >= 1 {
        state.set_reg(Reg(dst_start.0), tuple_value);
    }
    for i in 1..count as u16 {
        state.set_reg(Reg(dst_start.0 + i), Value::nil());
    }

    Ok(DispatchResult::Continue)
}

/// Pack (0x93) - Pack consecutive registers into a tuple (heap-allocated object).
///
/// Encoding: opcode + dst + src_start + count
/// Effect: Allocates a tuple with `count` elements from registers starting at `src_start`,
///         stores the result pointer in `dst`.
///
/// Tuple layout: ObjectHeader + [Value; count]
pub(in super::super) fn handle_pack(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src_start = read_reg(state)?;
    let count = read_u8(state)?;

    // Calculate size for the tuple data (array of Values)
    let data_size = count as usize * std::mem::size_of::<Value>();

    // Allocate on the heap with tuple type id
    let obj = state.heap.alloc_with_init(
        TypeId::TUPLE,
        data_size,
        |data| {
            // This closure doesn't have access to state, so we zero-init here
            // We'll write the actual values after allocation
            let _ = data;
        },
    )?;

    // Write values into the tuple data area
    let data_ptr = obj.data_ptr();
    for i in 0..count as usize {
        let value = state.get_reg(Reg(src_start.0 + i as u16));
        unsafe {
            let element_ptr = (data_ptr as *mut Value).add(i);
            std::ptr::write(element_ptr, value);
        }
    }

    // Store the object pointer in the destination register
    state.set_reg(dst, Value::from_ptr(obj.as_ptr()));
    Ok(DispatchResult::Continue)
}


// ============================================================================
// Dependent-type runtime packaging (T1-H)
// ============================================================================
//
// Pi / Sigma / Witness are 2-slot heap records that survive at Tier-0 so the
// interpreter can preserve enough structure for reflection tactics and for
// gradual verification boundaries. At Tier-1 (AOT) the static verifier elides
// them when the predicate / dependent-return-type obligation is discharged at
// compile time.
//
// Shared layout: `[header | slot0 | slot1]` where each slot is 8 bytes. The
// type_id on the heap header distinguishes the three:
//
//   TypeId::PI      (524) — slot0: captured param Value; slot1: return type id
//   TypeId::SIGMA   (525) — slot0: witness Value;        slot1: payload Value
//   TypeId::WITNESS (526) — slot0: refined value Value;  slot1: proof hash
//
// Projection onto these values piggybacks on the existing variant-payload
// accessors (GetVariantData field 0 / 1) because the slot layout matches the
// variant-payload convention (header + 8-byte tag/len word + value slots).
// Until dedicated projection opcodes land (`PiProj`, `SigmaFst`, `SigmaSnd`)
// the offsets here must stay aligned with `handle_get_variant_data`'s
// `payload_offset = OBJECT_HEADER_SIZE + 8`.

/// MakePi (0x8D) — pack a Π-value.
///
/// Encoding: opcode + dst:reg + param:reg + return_type_id:varint.
pub(in super::super) fn handle_make_pi(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let param_reg = read_reg(state)?;
    let return_type_id = read_varint(state)? as u32;

    let param_value = state.get_reg(param_reg);

    // Payload = 8 bytes reserved tag/len word + 2 × 8-byte slots. The first
    // slot is a Value (the captured param); the second stores the return type
    // id widened to u64 so it is bit-compatible with the Value slot width.
    let data_size = 8 + 2 * std::mem::size_of::<Value>();
    let obj = state.heap.alloc_with_init(
        TypeId::PI,
        data_size,
        |data| {
            // Write 0 at the tag word so it is never confused with a variant.
            let tag_ptr = data.as_mut_ptr() as *mut u32;
            unsafe {
                *tag_ptr = 0;
                *tag_ptr.add(1) = 2; // field_count, for observer helpers
            }
        },
    )?;
    state.record_allocation();

    let base_ptr = obj.as_ptr() as *mut u8;
    unsafe {
        let payload = base_ptr.add(heap::OBJECT_HEADER_SIZE + 8);
        std::ptr::write(payload as *mut Value, param_value);
        // The second slot holds the return type id. We store it as a u64
        // cast so a future projection opcode can read it via the same
        // Value-wide load path.
        std::ptr::write(
            payload.add(std::mem::size_of::<Value>()) as *mut u64,
            return_type_id as u64,
        );
    }

    state.set_reg(dst, Value::from_ptr(base_ptr));
    Ok(DispatchResult::Continue)
}

/// MakeSigma (0x8E) — pack a Σ-pair.
///
/// Encoding: opcode + dst:reg + witness:reg + payload:reg.
pub(in super::super) fn handle_make_sigma(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let witness_reg = read_reg(state)?;
    let payload_reg = read_reg(state)?;

    let witness = state.get_reg(witness_reg);
    let payload = state.get_reg(payload_reg);

    let data_size = 8 + 2 * std::mem::size_of::<Value>();
    let obj = state.heap.alloc_with_init(
        TypeId::SIGMA,
        data_size,
        |data| {
            let tag_ptr = data.as_mut_ptr() as *mut u32;
            unsafe {
                *tag_ptr = 0;
                *tag_ptr.add(1) = 2;
            }
        },
    )?;
    state.record_allocation();

    let base_ptr = obj.as_ptr() as *mut u8;
    unsafe {
        let slot0 = base_ptr.add(heap::OBJECT_HEADER_SIZE + 8);
        std::ptr::write(slot0 as *mut Value, witness);
        std::ptr::write(slot0.add(std::mem::size_of::<Value>()) as *mut Value, payload);
    }

    state.set_reg(dst, Value::from_ptr(base_ptr));
    Ok(DispatchResult::Continue)
}

/// MakeWitness (0x8F) — pack a refined value together with its proof hash.
///
/// Encoding: opcode + dst:reg + value:reg + proof_hash:varint.
pub(in super::super) fn handle_make_witness(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let value_reg = read_reg(state)?;
    let proof_hash = read_varint(state)? as u32;

    let value = state.get_reg(value_reg);

    let data_size = 8 + 2 * std::mem::size_of::<Value>();
    let obj = state.heap.alloc_with_init(
        TypeId::WITNESS,
        data_size,
        |data| {
            let tag_ptr = data.as_mut_ptr() as *mut u32;
            unsafe {
                *tag_ptr = 0;
                *tag_ptr.add(1) = 2;
            }
        },
    )?;
    state.record_allocation();

    let base_ptr = obj.as_ptr() as *mut u8;
    unsafe {
        let slot0 = base_ptr.add(heap::OBJECT_HEADER_SIZE + 8);
        std::ptr::write(slot0 as *mut Value, value);
        std::ptr::write(
            slot0.add(std::mem::size_of::<Value>()) as *mut u64,
            proof_hash as u64,
        );
    }

    state.set_reg(dst, Value::from_ptr(base_ptr));
    Ok(DispatchResult::Continue)
}
