//! CBGR (Capability-Based Generational References) instruction handlers for VBC interpreter.

use crate::instruction::{Opcode, Reg, CbgrSubOpcode};
use crate::types::TypeId;
use crate::value::{Value, FatRef, Capabilities};
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::super::heap;
use super::super::DispatchResult;
use super::bytecode_io::*;
use super::cbgr_helpers::{
    is_cbgr_ref, decode_cbgr_ref, encode_cbgr_ref, encode_cbgr_ref_mut,
    is_cbgr_ref_mutable, strip_cbgr_ref_mutability, validate_cbgr_generation,
    validate_epoch_window, CBGR_NO_CHECK_GENERATION, EPOCH_WINDOW_SIZE,
};
use verum_common::cbgr::caps;

// ============================================================================
// CBGR Reference Operations
// ============================================================================

/// Check if requested capabilities are compatible with mutability.
///
/// Each capability bit has different mutability requirements:
/// - READ/DELEGATE/NO_ESCAPE: always available
/// - WRITE/MUTABLE: requires is_mut
/// - EXECUTE: requires is_mut (function pointers need mutable access)
/// - REVOKE: requires is_mut (only owners can revoke)
/// - BORROWED: true when !is_mut
#[inline(always)]
fn check_capabilities_for_mutability(cap_mask: u32, is_mut: bool) -> bool {
    // Check each requested capability bit
    if (cap_mask & caps::WRITE) != 0 && !is_mut {
        return false;
    }
    if (cap_mask & caps::MUTABLE) != 0 && !is_mut {
        return false;
    }
    if (cap_mask & caps::REVOKE) != 0 && !is_mut {
        return false;
    }
    // BORROWED bit is set for immutable refs - check if they want BORROWED on mutable ref
    if (cap_mask & caps::BORROWED) != 0 && is_mut {
        // Actually, BORROWED means "non-owning" - mutable refs CAN be borrowed
        // This check might be too strict; let's allow it
    }
    // READ, DELEGATE, NO_ESCAPE, EXECUTE are always available
    true
}

/// Ref (0x70) - Create immutable reference (Tier 0 - full validation).
///
/// For interpreter mode, an immutable reference stores the absolute register index
/// and current CBGR generation of the referenced variable. On dereference, the
/// generation is validated to detect use-after-free.
pub(in super::super) fn handle_ref_create(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;

    // Check if this RefCreate follows a CBGR data deref. If so, the Deref already
    // read the stored Value and we need to create a pointer reference back to the
    // original CBGR data location (for patterns like `&*value` which should yield
    // a raw pointer to the data, not a register-based reference).
    if let Some((deref_dst_reg, data_ptr_addr)) = state.cbgr_deref_source.take()
        && src.0 == deref_dst_reg {
            // Source register matches the Deref destination - restore the CBGR pointer
            // Record creation epoch for this pointer-based reference so .epoch()
            // returns the reference creation time, not the allocation time.
            state.cbgr_ref_creation_epoch.insert(data_ptr_addr, state.cbgr_epoch);
            // RefCreate (0x70) creates immutable references, so remove from mutable set.
            // This implements capability downgrade: &*mut_ref yields an immutable ref.
            state.cbgr_mutable_ptrs.remove(&data_ptr_addr);
            state.set_reg(dst, Value::from_ptr(data_ptr_addr as *mut u8));
            return Ok(DispatchResult::Continue);
        }

    // Always create a CBGR register reference.
    // This ensures consistent behavior with RefMut and enables proper
    // dereference semantics for all value types including structs.
    let abs_index = (state.reg_base() + src.0 as u32) as u32;
    let generation = state.registers.get_generation(abs_index);
    if state.config.count_instructions {
        state.stats.cbgr_stats.tier0_refs += 1;
    }
    state.set_reg(dst, Value::from_i64(encode_cbgr_ref(abs_index, generation)));
    Ok(DispatchResult::Continue)
}

/// RefMut (0x71) - Create mutable reference (Tier 0 - full validation).
///
/// Stores the absolute register index and CBGR generation so that DerefMut
/// can validate the reference before writing back to the original variable.
/// Encodes the mutability bit so epoch_caps/can_write can detect mutable refs.
///
/// IMPORTANT: Always creates a CBGR register reference, even for pointer-valued
/// variables (structs). This ensures DerefMut can update the register value,
/// not just write to the heap memory the pointer points to.
pub(in super::super) fn handle_ref_mut(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;

    // Handle CBGR deref-source pattern: `&mut *value` creates a pointer-based
    // mutable reference back to the original heap data location.
    if let Some((deref_dst_reg, data_ptr_addr)) = state.cbgr_deref_source.take()
        && src.0 == deref_dst_reg {
            state.cbgr_ref_creation_epoch.insert(data_ptr_addr, state.cbgr_epoch);
            state.set_reg(dst, Value::from_ptr(data_ptr_addr as *mut u8));
            state.cbgr_mutable_ptrs.insert(data_ptr_addr);
            return Ok(DispatchResult::Continue);
        }

    // Check if the source register already contains a CBGR mutable reference.
    // This happens in nested method calls where an outer method with `&mut self`
    // calls another method on `self` that also takes `&mut self`. The codegen
    // emits RefMut for the inner call, but `self` is already a CBGR reference.
    // In this case, we pass through the existing reference directly instead of
    // creating a reference-to-reference which would cause NullPointer errors.
    let src_val = state.get_reg(src);
    if is_cbgr_ref(&src_val) && is_cbgr_ref_mutable(src_val.as_i64()) {
        // Source is already a mutable CBGR reference - pass it through directly
        state.set_reg(dst, src_val);
        return Ok(DispatchResult::Continue);
    }

    // Always create a CBGR register reference for RefMut.
    // This ensures that DerefMut will update the register value, which is
    // essential for struct assignment: `*ref = new_struct` must update the
    // register containing the struct pointer, not write into the struct's memory.
    //
    // Previous behavior passed pointers through directly, which broke
    // full struct assignment through mutable references.
    let abs_index = (state.reg_base() + src.0 as u32) as u32;
    let generation = state.registers.get_generation(abs_index);
    if state.config.count_instructions {
        state.stats.cbgr_stats.tier0_refs += 1;
    }
    state.set_reg(dst, Value::from_i64(encode_cbgr_ref_mut(abs_index, generation)));
    Ok(DispatchResult::Continue)
}

/// Deref (0x72) - Dereference with CBGR validation (Tier 0).
///
/// Reads the value at the absolute register index stored in the reference.
/// For Tier 0 references, validates the CBGR generation to detect use-after-free.
pub(in super::super) fn handle_deref(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let ref_val = state.get_reg(src);
    if state.config.count_instructions {
        state.stats.cbgr_stats.tier0_derefs += 1;
    }

    // Handle ThinRef values (CBGR references stored in global table)
    if ref_val.is_thin_ref() {
        let thin_ref = ref_val.as_thin_ref();
        if !thin_ref.is_null() {
            // Read Value from the memory location
            let value = unsafe { *(thin_ref.ptr as *const Value) };
            state.set_reg(dst, value);
        } else {
            state.set_reg(dst, Value::nil());
        }
        return Ok(DispatchResult::Continue);
    }

    // Handle FatRef values (CBGR references with metadata)
    if ref_val.is_fat_ref() {
        let fat_ref = ref_val.as_fat_ref();
        if !fat_ref.is_null() {
            // Read Value from the memory location
            let value = unsafe { *(fat_ref.ptr() as *const Value) };
            state.set_reg(dst, value);
        } else {
            state.set_reg(dst, Value::nil());
        }
        return Ok(DispatchResult::Continue);
    }

    if ref_val.is_ptr() && !ref_val.is_nil() {
        let base_ptr = ref_val.as_ptr::<u8>();
        if !base_ptr.is_null() {
            let ptr_addr = base_ptr as usize;
            let header_addr = ptr_addr.wrapping_sub(32); // 32-byte AllocationHeader
            if state.cbgr_allocations.contains(&header_addr) {
                // CBGR data pointer: check if allocation has been freed
                // Layout: [size:4][align:4][generation:4][epoch:2][caps:2][type_id:4][flags:4][reserved:8]
                let flags = unsafe { *((header_addr + 20) as *const u32) };
                if flags & 1 != 0 {
                    return Err(InterpreterError::Panic {
                        message: "CBGR use-after-free detected".to_string(),
                    });
                }
                // Read the stored Value from memory.
                // Track source so a subsequent RefCreate can create a pointer
                // reference back to this location instead of a register ref.
                let value = unsafe { *(base_ptr as *const Value) };
                state.cbgr_deref_source = Some((dst.0, ptr_addr));
                state.set_reg(dst, value);
            } else if state.cbgr_allocations.contains(&ptr_addr) {
                // CBGR base pointer (AllocationHeader): identity deref for struct access
                state.set_reg(dst, ref_val);
            } else if state.cbgr_mutable_ptrs.contains(&ptr_addr) {
                // Variant field pointer (from GetVariantDataRef): read the Value from memory.
                // This enables ref/ref mut pattern bindings to work correctly.
                let value = unsafe { *(base_ptr as *const Value) };
                state.set_reg(dst, value);
            } else {
                // Regular heap object dereference: identity deref (return pointer as-is).
                // Sum type variants and other heap objects should NOT be automatically unwrapped.
                // The pattern matching (IsVar, GetVariantData) handles variant extraction.
                // Heap<T> wrappers are handled explicitly by codegen via GetVariantData.
                //
                // Previous bug: automatically unwrapping single-field variants broke sum types
                // like IpAddr where V4(Ipv4Addr) and V6(Ipv6Addr) both have field_count=1.
                // This caused `match *self` to receive the inner type instead of the variant.
                state.set_reg(dst, ref_val);
            }
        } else {
            state.set_reg(dst, Value::nil());
        }
    } else if is_cbgr_ref(&ref_val) {
        // Register-based reference: decode abs_index and generation
        let (abs_index, generation) = decode_cbgr_ref(ref_val.as_i64());
        // CBGR generation validation (Tier 0 only; skipped for Tier 1/2 sentinel)
        validate_cbgr_generation(state, abs_index, generation)?;
        let value = state.registers.get_absolute(abs_index);
        state.set_reg(dst, value);
    } else {
        // Fallback: return the value as-is (e.g., for unit types, nil, or plain integers)
        state.set_reg(dst, ref_val);
    }

    Ok(DispatchResult::Continue)
}

/// DerefMut (0x73) - Write through mutable reference (Tier 0).
///
/// Writes the value to the absolute register index stored in the reference.
/// This enables mutation through &mut parameters. Validates CBGR generation
/// before writing to detect use-after-free.
pub(in super::super) fn handle_deref_mut(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let ref_reg = read_reg(state)?;
    let value_reg = read_reg(state)?;
    let ref_val = state.get_reg(ref_reg);
    let value = state.get_reg(value_reg);
    if state.config.count_instructions {
        state.stats.cbgr_stats.tier0_derefs += 1;
    }

    // Handle ThinRef values (CBGR references stored in global table)
    if ref_val.is_thin_ref() {
        let thin_ref = ref_val.as_thin_ref();
        if !thin_ref.is_null() {
            // Write Value to the memory location
            unsafe { std::ptr::write(thin_ref.ptr as *mut Value, value); }
            // Advance CBGR epoch on mutation
            state.cbgr_epoch = state.cbgr_epoch.wrapping_add(1);
        }
        return Ok(DispatchResult::Continue);
    }

    // Handle FatRef values (CBGR references with metadata)
    if ref_val.is_fat_ref() {
        let fat_ref = ref_val.as_fat_ref();
        if !fat_ref.is_null() {
            // Write Value to the memory location
            unsafe { std::ptr::write(fat_ref.ptr() as *mut Value, value); }
            // Advance CBGR epoch on mutation
            state.cbgr_epoch = state.cbgr_epoch.wrapping_add(1);
        }
        return Ok(DispatchResult::Continue);
    }

    // The reference holds a negative-encoded absolute register index — write through it
    if is_cbgr_ref(&ref_val) {
        let (abs_index, generation) = decode_cbgr_ref(ref_val.as_i64());
        validate_cbgr_generation(state, abs_index, generation)?;
        state.registers.set_absolute(abs_index, value);
        // CBGR epoch advancement: mutation through reference advances the epoch
        // This enables temporal ordering detection for stale references
        state.cbgr_epoch = state.cbgr_epoch.wrapping_add(1);
    } else if ref_val.is_ptr() && !ref_val.is_nil() {
        // Heap pointer deref-mut: write value at pointer location
        let ptr = ref_val.as_ptr::<Value>();
        unsafe { std::ptr::write(ptr, value); }
        // CBGR epoch advancement on heap mutation
        state.cbgr_epoch = state.cbgr_epoch.wrapping_add(1);
        // Update the epoch in the AllocationHeader for this allocation
        // AllocationHeader is 32 bytes before the data pointer
        // Layout: [size:4][align:4][generation:4][epoch:2][caps:2][type_id:4][flags:4][reserved:8]
        let ptr_addr = ptr as usize;
        let header_addr = ptr_addr.wrapping_sub(32);
        if state.cbgr_allocations.contains(&header_addr) {
            unsafe {
                let epoch_ptr = (header_addr + 12) as *mut u16;
                *epoch_ptr = state.cbgr_epoch as u16;
            }
        }
    }
    Ok(DispatchResult::Continue)
}

/// ChkRef (0x74) - Check reference validity (Tier 0 CBGR validation).
///
/// Validates the CBGR generation and epoch of a reference.
/// Supports both register-based and heap-based CBGR references.
///
/// If the generation has been bumped (variable went out of scope) or
/// the epoch has advanced (generation wrapped around), this panics
/// with a use-after-free error.
pub(in super::super) fn handle_chk_ref(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let ref_reg = read_reg(state)?;
    let ref_val = state.get_reg(ref_reg);

    if is_cbgr_ref(&ref_val) {
        // Register-based CBGR reference
        let (abs_index, generation) = decode_cbgr_ref(ref_val.as_i64());
        validate_cbgr_generation(state, abs_index, generation)?;
    } else if ref_val.is_ptr() && !ref_val.is_nil() {
        // Heap-based CBGR reference - validate AllocationHeader
        let ptr_addr = ref_val.as_ptr::<u8>() as usize;
        let header_addr = ptr_addr.wrapping_sub(32); // 32-byte AllocationHeader

        if state.cbgr_allocations.contains(&header_addr) {
            // Read generation and flags from AllocationHeader
            // Layout: [size:4][align:4][generation:4][epoch:2][caps:2][type_id:4][flags:4]
            let generation = unsafe { *((header_addr + 8) as *const u32) };
            let _epoch = unsafe { *((header_addr + 12) as *const u16) };
            let flags = unsafe { *((header_addr + 20) as *const u32) };

            // Check if allocation has been freed
            if flags & 1 != 0 {
                return Err(InterpreterError::Panic {
                    message: "CBGR use-after-free: allocation has been freed".to_string(),
                });
            }

            // Check epoch against reference creation epoch
            if let Some(&ref_epoch) = state.cbgr_ref_creation_epoch.get(&ptr_addr) {
                let current_epoch = state.cbgr_epoch;
                // Allow some drift but detect major epoch skips
                if current_epoch.wrapping_sub(ref_epoch) > 0x1000_0000 {
                    return Err(InterpreterError::Panic {
                        message: format!(
                            "CBGR epoch mismatch: reference epoch {}, current {}",
                            ref_epoch, current_epoch
                        ),
                    });
                }
            }

            // Validate generation hasn't changed unexpectedly
            // (This would indicate the object was deallocated and reallocated)
            if generation == 0 {
                return Err(InterpreterError::Panic {
                    message: "CBGR validation failed: invalid generation (0)".to_string(),
                });
            }
        }
    }

    if state.config.count_instructions {
        state.stats.cbgr_stats.cbgr_checks += 1;
    }
    Ok(DispatchResult::Continue)
}

/// RefChecked (0x75) - Create Tier 1 checked reference.
///
/// Tier 1 references are compiler-proven safe and skip generation checks.
/// Uses CBGR_NO_CHECK_GENERATION sentinel so deref skips validation.
pub(in super::super) fn handle_ref_checked(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let abs_index = (state.reg_base() + src.0 as u32) as u32;
    if state.config.count_instructions {
        state.stats.cbgr_stats.tier1_refs += 1;
    }
    state.set_reg(dst, Value::from_i64(encode_cbgr_ref(abs_index, CBGR_NO_CHECK_GENERATION)));
    Ok(DispatchResult::Continue)
}

/// RefUnsafe (0x76) - Create Tier 2 unsafe reference (no runtime checks).
///
/// Tier 2 references require manual safety proof and skip generation checks.
/// Uses CBGR_NO_CHECK_GENERATION sentinel so deref skips validation.
pub(in super::super) fn handle_ref_unsafe(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let abs_index = (state.reg_base() + src.0 as u32) as u32;
    if state.config.count_instructions {
        state.stats.cbgr_stats.tier2_refs += 1;
    }
    state.set_reg(dst, Value::from_i64(encode_cbgr_ref(abs_index, CBGR_NO_CHECK_GENERATION)));
    Ok(DispatchResult::Continue)
}

/// DropRef (0x77) - Drop a value/reference.
///
/// If the value has a user-defined Drop implementation, calls the drop method first.
/// Then bumps the CBGR generation for the register slot, invalidating any
/// references that captured the old generation. For CBGR heap allocations,
/// also bumps the generation in the AllocationHeader.
///
/// The drop implementation works as follows:
/// 1. First call: if value has Drop impl, set up drop call, clear register, return Continue
/// 2. Drop function executes and returns to this instruction
/// 3. Second call: register is now unit (cleared), skip drop call, do CBGR cleanup
pub(in super::super) fn handle_drop_ref(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let src = read_reg(state)?;
    let mut val = state.get_reg(src);

    // If the source register is already cleared (unit), check for pending field drops
    if val.is_unit() && !state.pending_drops.is_empty() {
        // SAFETY: is_empty() check above guarantees pop() returns Some
        val = match state.pending_drops.pop() {
            Some(v) => v,
            None => return Ok(DispatchResult::Continue),
        };
        // DEBUG: eprintln!("[DEBUG DropRef] Processing pending field drop: {:?}", val);
    }

    // Check for user-defined Drop implementation on heap objects with ObjectHeader
    // Use is_regular_ptr() to exclude generators, ThinRefs, and other special pointer types
    if val.is_regular_ptr() {
        let obj_ptr = val.as_ptr::<u8>();

        // Only check standard heap objects (not CBGR allocations which have 32-byte AllocationHeader)
        let is_cbgr_alloc = state.cbgr_allocations.contains(&(obj_ptr as usize).wrapping_sub(32));

        if !is_cbgr_alloc {
            // Standard heap objects: the pointer points directly to the ObjectHeader
            // (see handle_new which stores obj.as_ptr() - the header pointer)
            let header_ptr = obj_ptr;

            // Read type_id from ObjectHeader
            let type_id = unsafe {
                let header = header_ptr as *const heap::ObjectHeader;
                (*header).type_id
            };

            // Debug: show what type we're dropping
            if type_id.0 >= crate::types::TypeId::FIRST_USER {
                let type_idx = (type_id.0 - crate::types::TypeId::FIRST_USER) as usize;
                if let Some(type_desc) = state.module.types.get(type_idx) {
                    let _type_name = state.module.strings.get(type_desc.name).unwrap_or("?");
                    // DEBUG: eprintln!("[DEBUG DropRef] Dropping type '{}' (id={}, idx={}, drop_fn={:?}, fields={})",
                    //     type_name, type_id.0, type_idx, type_desc.drop_fn, type_desc.fields.len());
                }
            }

            // Look up TypeDescriptor to find drop_fn
            // Extract all needed values before any mutable operations to avoid borrow conflicts
            let drop_info = if type_id.0 >= crate::types::TypeId::FIRST_USER {
                let type_idx = (type_id.0 - crate::types::TypeId::FIRST_USER) as usize;
                state.module.types.get(type_idx)
                    .and_then(|type_desc| type_desc.drop_fn)
                    .and_then(|drop_fn_id| {
                        state.module.functions.get(drop_fn_id as usize)
                            .map(|func| (drop_fn_id, func.register_count, func.bytecode_offset))
                    })
            } else {
                None
            };

            if let Some((drop_fn_id, reg_count, _bytecode_offset)) = drop_info {
                // Set return_pc to the CURRENT DropRef instruction.
                // After drop() returns, we'll re-execute DropRef with a cleared register.
                // Subtract the instruction size to re-execute this instruction
                // (DropRef encoding: opcode(1) + reg(1) = 2 bytes typically)
                let current_pc = state.pc();
                let return_pc = current_pc.saturating_sub(2);
                let caller_base = state.reg_base();

                // Push a new frame for the drop call
                let func_id = crate::module::FunctionId(drop_fn_id);
                let new_base = state.call_stack.push_frame(func_id, reg_count, return_pc, src)?;
                state.registers.push_frame(reg_count);

                // Set r0 to the value being dropped (as &mut self)
                // Use new_base to set the register in the new frame
                state.registers.set(new_base, Reg(0), val);

                // Clear the source register in caller's frame AFTER setting up r0 in callee
                // This prevents infinite loops when DropRef re-executes
                state.registers.set(caller_base, src, Value::unit());

                // Set PC to 0 (start of function)
                // The bytecode_offset is used internally when reading bytes
                state.set_pc(0);
                state.record_call();

                // Return Continue to execute the drop function
                // When it returns, DropRef will re-execute but val will be unit
                return Ok(DispatchResult::Continue);
            } else {
                // No drop_fn for this type, but check if it has fields with Drop impls
                // This handles structs like StructWithTrackers whose fields have Drop
                if type_id.0 >= crate::types::TypeId::FIRST_USER {
                    let type_idx = (type_id.0 - crate::types::TypeId::FIRST_USER) as usize;
                    if let Some(type_desc) = state.module.types.get(type_idx) {
                        // Check each field for droppable types
                        for field in &type_desc.fields {
                            // Get the field type ID
                            let field_type_id = match &field.type_ref {
                                crate::types::TypeRef::Concrete(tid) => Some(*tid),
                                _ => None,
                            };

                            if let Some(ftid) = field_type_id
                                && ftid.0 >= crate::types::TypeId::FIRST_USER {
                                    let field_type_idx = (ftid.0 - crate::types::TypeId::FIRST_USER) as usize;
                                    let has_drop = state.module.types.get(field_type_idx)
                                        .map(|fd| fd.drop_fn.is_some())
                                        .unwrap_or(false);

                                    if has_drop {
                                        // Read the field value from the struct
                                        // Struct layout: [ObjectHeader][field0][field1][...]
                                        let field_ptr = unsafe {
                                            obj_ptr.add(heap::OBJECT_HEADER_SIZE + field.offset as usize) as *const Value
                                        };
                                        let field_val = unsafe { *field_ptr };
                                        // DEBUG: eprintln!("[DEBUG DropRef] Queueing field '{}' for drop: {:?}",
                                        //     state.module.strings.get(field.name).unwrap_or("?"), field_val);
                                        state.pending_drops.push(field_val);
                                    }
                                }
                        }

                        // If we queued any pending drops, process the first one now
                        if !state.pending_drops.is_empty() {
                            // Clear the original register to prevent re-processing
                            state.set_reg(src, Value::unit());

                            // Re-run DropRef to process the pending drops
                            let current_pc = state.pc();
                            state.set_pc(current_pc.saturating_sub(2));
                            return Ok(DispatchResult::Continue);
                        }
                    }
                }
            }
        }
    }

    // Handle tuple drops - iterate through elements and drop each
    // Use is_regular_ptr() to exclude generators, ThinRefs, and other special pointer types
    if val.is_regular_ptr() {
        let obj_ptr = val.as_ptr::<u8>();
        let is_cbgr_alloc = state.cbgr_allocations.contains(&(obj_ptr as usize).wrapping_sub(32));

        if !is_cbgr_alloc {
            let header = unsafe { &*(obj_ptr as *const heap::ObjectHeader) };
            if header.type_id == TypeId::TUPLE {
                // Tuple layout: [ObjectHeader][elem0][elem1][elem2]...
                // Size is in bytes, elements are sizeof(Value) each
                let elem_count = header.size as usize / std::mem::size_of::<Value>();
                let data_ptr = unsafe { obj_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };

                // DEBUG: eprintln!("[DEBUG DropRef] Dropping TUPLE with {} elements", elem_count);

                // Queue elements in REVERSE order so that after LIFO processing,
                // they're dropped in forward order (element 0 first, then 1, etc.)
                for i in (0..elem_count).rev() {
                    let elem = unsafe { *data_ptr.add(i) };
                    if elem.is_ptr() && !elem.is_nil() {
                        // DEBUG: eprintln!("[DEBUG DropRef] Queueing tuple element {} for drop: {:?}", i, elem);
                        state.pending_drops.push(elem);
                    }
                }

                // If we queued any pending drops, process them
                if !state.pending_drops.is_empty() {
                    // Clear the source register
                    state.set_reg(src, Value::unit());

                    // Re-run DropRef to process the pending drops
                    let current_pc = state.pc();
                    state.set_pc(current_pc.saturating_sub(2));
                    return Ok(DispatchResult::Continue);
                }
            }
        }
    }

    // For CBGR heap allocations (pointer to data after AllocationHeader),
    // bump the generation in the header to invalidate references.
    // Layout: [size:4][align:4][generation:4][epoch:2][caps:2][type_id:4][flags:4][reserved:8]
    if val.is_ptr() && !val.is_nil() {
        let data_ptr = val.as_ptr::<u8>() as usize;
        let header_addr = data_ptr.wrapping_sub(32); // 32-byte AllocationHeader
        if state.cbgr_allocations.contains(&header_addr) {
            // Bump generation and set freed flag in AllocationHeader
            unsafe {
                let gen_ptr = (header_addr + 8) as *mut u32; // generation at offset 8
                *gen_ptr = (*gen_ptr).wrapping_add(1);
                let flags_ptr = (header_addr + 20) as *mut u32; // flags at offset 20
                *flags_ptr |= 1; // FREED flag
            }
            // Advance global epoch on deallocation
            state.cbgr_epoch = state.cbgr_epoch.wrapping_add(1);
        }
    }

    // Bump the register slot generation to invalidate register-based references
    let abs_index = (state.reg_base() + src.0 as u32) as u32;
    state.registers.bump_generation(abs_index);

    // Clear the register
    state.set_reg(src, Value::unit());
    Ok(DispatchResult::Continue)
}

/// CbgrExtended (0x78) - Extended CBGR (Capability-Based Generational References) operations.
///
/// Format: `[0x78] [sub_opcode:u8] [operands...]`
///
/// Sub-opcode categories:
/// - 0x00-0x0F: Slice and Interior References
/// - 0x10-0x1F: Capability Operations
/// - 0x20-0x2F: Generation and Epoch Operations
/// - 0x30-0x3F: Reference Conversion
/// - 0x40-0x4F: Debug and Introspection
///
/// Note: The interpreter provides simplified implementations for these operations.
/// The AOT compiler generates optimized code with full CBGR semantics.
pub(in super::super) fn handle_cbgr_extended(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let sub_op_byte = read_u8(state)?;
    let sub_op = CbgrSubOpcode::from_byte(sub_op_byte);

    match sub_op {
        // ================================================================
        // Slice and Interior References (0x00-0x0F)
        // ================================================================
        Some(CbgrSubOpcode::RefSlice) => {
            // Create slice reference (FatRef) from array/buffer
            // Format: dst:reg, src:reg, start:reg, len:reg
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let start_reg = read_reg(state)?;
            let len_reg = read_reg(state)?;

            let src = state.get_reg(src_reg);
            let start = state.get_reg(start_reg).as_i64() as usize;
            let len = state.get_reg(len_reg).as_i64() as u64;

            // eprintln!("[DEBUG RefSlice] src={:?}, start={}, len={}", src, start, len);
            // eprintln!("[DEBUG RefSlice] src.is_ptr()={}, src.is_thin_ref()={}, src.is_fat_ref()={}",
            //          src.is_ptr(), src.is_thin_ref(), src.is_fat_ref());

            // Get the base pointer from source - could be a pointer, thin ref, or object
            let mut base_ptr = if src.is_ptr() {
                // eprintln!("[DEBUG RefSlice] src is pointer: {:p}", src.as_ptr::<u8>());
                src.as_ptr::<u8>()
            } else if src.is_thin_ref() {
                let tr = src.as_thin_ref();
                // eprintln!("[DEBUG RefSlice] src is thin_ref: {:p}", tr.ptr);
                tr.ptr
            } else if src.is_fat_ref() {
                let fr = src.as_fat_ref();
                // eprintln!("[DEBUG RefSlice] src is fat_ref: {:p}", fr.ptr());
                fr.ptr()
            } else {
                // eprintln!("[DEBUG RefSlice] src is none of the above, using null");
                // Fallback: treat as null for non-pointer values
                std::ptr::null_mut()
            };

            // If base_ptr is a List object, follow backing_ptr to get actual data
            if !base_ptr.is_null() {
                let header = unsafe { &*(base_ptr as *const heap::ObjectHeader) };
                // eprintln!("[DEBUG RefSlice] base_ptr={:p}, type_id={:?}", base_ptr, header.type_id);
                if header.type_id == TypeId::LIST {
                    // List layout: [ObjectHeader][len: Value][cap: Value][backing_ptr: Value]
                    // backing_ptr points to another array object with the actual elements
                    let _len_val = unsafe {
                        *(base_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value)
                    };
                    // eprintln!("[DEBUG RefSlice] list len_val={:?}", len_val);
                    let backing_ptr_val = unsafe {
                        *(base_ptr.add(heap::OBJECT_HEADER_SIZE + 16) as *const Value)
                    };
                    // eprintln!("[DEBUG RefSlice] backing_ptr_val={:?}, is_ptr={}", backing_ptr_val, backing_ptr_val.is_ptr());
                    if backing_ptr_val.is_ptr() && !backing_ptr_val.is_nil() {
                        let backing_array = backing_ptr_val.as_ptr::<u8>();
                        // eprintln!("[DEBUG RefSlice] backing_array={:p}", backing_array);
                        // The backing array also has an ObjectHeader, skip it to get elements
                        base_ptr = unsafe { backing_array.add(heap::OBJECT_HEADER_SIZE) };
                        // eprintln!("[DEBUG RefSlice] new base_ptr (after skipping header)={:p}", base_ptr);
                        // Debug: print first element
                        let _first_elem = unsafe { *(base_ptr as *const Value) };
                        // eprintln!("[DEBUG RefSlice] first element at data start: {:?}", first_elem);
                    }
                } else {
                    // Non-LIST typed arrays (e.g., [Int; 3] allocated with TypeId::U64)
                    // These have layout: [ObjectHeader][data...]
                    // We need to skip past the header to get to the data
                    // eprintln!("[DEBUG RefSlice] non-LIST array, skipping header");
                    base_ptr = unsafe { base_ptr.add(heap::OBJECT_HEADER_SIZE) };
                    // eprintln!("[DEBUG RefSlice] new base_ptr (after skipping header)={:p}", base_ptr);
                    // Debug: print first element as raw i64
                    let _first_elem = unsafe { *(base_ptr as *const i64) };
                    // eprintln!("[DEBUG RefSlice] first element at data start (raw i64): {}", first_elem);
                }
            }

            // Determine element size based on source TypeId
            // For typed arrays (U8, U16, U32, U64), elements are stored as raw integers
            // For LIST and other types, elements are NaN-boxed Values (elem_size = 0 signals Value)
            let elem_size: u32 = if !src.is_ptr() || src.is_nil() {
                0 // Default to Value
            } else {
                let src_ptr = src.as_ptr::<u8>();
                if src_ptr.is_null() {
                    0
                } else {
                    let src_header = unsafe { &*(src_ptr as *const heap::ObjectHeader) };
                    match src_header.type_id {
                        TypeId::U8 => 1,
                        TypeId::U16 => 2,
                        TypeId::U32 => 4,
                        TypeId::U64 => 8,
                        _ => 0, // LIST, UNIT, etc. use NaN-boxed Values
                    }
                }
            };
            // eprintln!("[DEBUG RefSlice] elem_size={}", elem_size);

            // Adjust pointer by start offset based on element size
            let actual_elem_size = if elem_size == 0 { std::mem::size_of::<Value>() } else { elem_size as usize };
            let slice_ptr = if !base_ptr.is_null() {
                unsafe { base_ptr.add(start * actual_elem_size) }
            } else {
                base_ptr
            };

            // Create FatRef with slice pointer and length as metadata
            // Use generation=0 and current epoch for interpreter simplicity
            // Store elem_size in reserved field: 0 = Value, 1/2/4/8 = raw integer size
            let mut fat_ref = FatRef::slice(
                slice_ptr,
                0, // generation (not tracked in interpreter)
                (state.cbgr_epoch & 0xFFFF) as u16,
                Capabilities::MUT_EXCLUSIVE, // Full capabilities for slices
                len,
            );
            fat_ref.reserved = elem_size;

            state.set_reg(dst, Value::from_fat_ref(fat_ref));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::RefInterior) => {
            // Create interior reference to struct field
            // Format: dst:reg, base:reg, field_offset:u32
            let dst = read_reg(state)?;
            let base_reg = read_reg(state)?;
            let field_offset = read_u32(state)?;

            let base = state.get_reg(base_reg);
            if base.is_ptr() && !base.is_nil() {
                let base_ptr = base.as_ptr::<u8>();
                let field_ptr = unsafe { base_ptr.add(field_offset as usize) };
                state.set_reg(dst, Value::from_ptr(field_ptr));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::RefArrayElement) => {
            // Create interior reference to array element
            // Format: dst:reg, base:reg, index:reg
            let dst = read_reg(state)?;
            let base_reg = read_reg(state)?;
            let index_reg = read_reg(state)?;

            let base = state.get_reg(base_reg);
            let index = state.get_reg(index_reg).as_i64() as usize;

            if base.is_ptr() && !base.is_nil() {
                let base_ptr = base.as_ptr::<u8>();
                // Assume Value-sized elements (8 bytes)
                let elem_ptr = unsafe { base_ptr.add(index * std::mem::size_of::<Value>()) };
                state.set_reg(dst, Value::from_ptr(elem_ptr));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::RefTrait) => {
            // Create reference to trait object (fat pointer with vtable)
            // Format: dst:reg, src:reg, vtable_id:u32
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let _vtable_id = read_u32(state)?;

            // Simplified: just pass through the reference
            let src = state.get_reg(src_reg);
            state.set_reg(dst, src);
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::Unslice) => {
            // Get underlying pointer from slice reference
            // Format: dst:reg, slice_ref:reg
            let dst = read_reg(state)?;
            let slice_reg = read_reg(state)?;

            let slice = state.get_reg(slice_reg);
            // Extract pointer from the FatRef (slice)
            let ptr_value = if slice.is_fat_ref() {
                // For FatRef (slice), extract the pointer from the thin ref portion
                let fat_ref = slice.as_fat_ref();
                Value::from_ptr(fat_ref.ptr())
            } else if slice.is_thin_ref() {
                // For ThinRef, extract the pointer directly
                let thin_ref = slice.as_thin_ref();
                Value::from_ptr(thin_ref.ptr)
            } else if slice.is_ptr() {
                // Already a raw pointer, just pass through
                slice
            } else {
                // For non-reference types, return null pointer
                Value::from_ptr(std::ptr::null_mut::<u8>())
            };
            state.set_reg(dst, ptr_value);
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::SliceLen) => {
            // Get slice length from FatRef
            // Format: dst:reg, slice_ref:reg
            let dst = read_reg(state)?;
            let slice_reg = read_reg(state)?;

            // Get slice value and extract length from FatRef
            let slice = state.get_reg(slice_reg);
            let len = if slice.is_fat_ref() {
                slice.as_fat_ref().len() as i64
            } else {
                // For non-FatRef values, return 0 (or could be error)
                0
            };
            state.set_reg(dst, Value::from_i64(len));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::SliceGet) => {
            // Get element at index from slice (bounds-checked)
            // Format: dst:reg, slice_ref:reg, index:reg
            let dst = read_reg(state)?;
            let slice_reg = read_reg(state)?;
            let index_reg = read_reg(state)?;

            let slice = state.get_reg(slice_reg);
            let index = state.get_reg(index_reg).as_i64() as usize;

            let value = if slice.is_fat_ref() {
                let fat_ref = slice.as_fat_ref();
                let len = fat_ref.len() as usize;
                if index < len {
                    // Read element from memory at offset
                    let ptr = fat_ref.ptr() as *const Value;
                    // SAFETY: Index is bounds-checked above
                    unsafe { *ptr.add(index) }
                } else {
                    return Err(crate::interpreter::InterpreterError::IndexOutOfBounds {
                        index: index as i64,
                        length: len,
                    });
                }
            } else {
                Value::nil()
            };
            state.set_reg(dst, value);
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::SliceGetUnchecked) => {
            // Get element at index from slice (unchecked)
            // Format: dst:reg, slice_ref:reg, index:reg
            let dst = read_reg(state)?;
            let slice_reg = read_reg(state)?;
            let index_reg = read_reg(state)?;

            let slice = state.get_reg(slice_reg);
            let index = state.get_reg(index_reg).as_i64() as usize;

            // SAFETY: Unchecked access - assumes index is valid
            let value = if slice.is_fat_ref() {
                let fat_ref = slice.as_fat_ref();
                let ptr = fat_ref.ptr() as *const Value;
                // SAFETY: No bounds check - caller must ensure index is valid
                unsafe { *ptr.add(index) }
            } else {
                Value::nil()
            };
            state.set_reg(dst, value);
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::SliceSubslice) => {
            // Create subslice from existing slice
            // Format: dst:reg, src:reg, start:reg, end:reg
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let start_reg = read_reg(state)?;
            let end_reg = read_reg(state)?;

            let src = state.get_reg(src_reg);
            let start = state.get_reg(start_reg).as_i64() as u64;
            let end = state.get_reg(end_reg).as_i64() as u64;

            // Create new FatRef with adjusted pointer and length
            let result = if src.is_fat_ref() {
                let fat_ref = src.as_fat_ref();
                let len = fat_ref.len();
                if start <= end && end <= len {
                    // Create new FatRef pointing to subslice
                    let element_size = std::mem::size_of::<Value>();
                    let new_ptr = unsafe { (fat_ref.ptr() as *const u8).add(start as usize * element_size) };
                    let new_len = end - start;
                    // Create a new FatRef with updated pointer and length
                    let new_fat_ref = crate::value::FatRef::new(
                        new_ptr as *mut u8,
                        fat_ref.generation(),
                        fat_ref.epoch(),
                        fat_ref.capabilities(),
                        new_len,
                    );
                    Value::from_fat_ref(new_fat_ref)
                } else {
                    return Err(crate::interpreter::InterpreterError::IndexOutOfBounds {
                        index: end as i64,
                        length: len as usize,
                    });
                }
            } else {
                src
            };
            state.set_reg(dst, result);
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::SliceSplitAt) => {
            // Split slice at index into two slices
            // Format: dst1:reg, dst2:reg, src:reg, mid:reg
            let dst1 = read_reg(state)?;
            let dst2 = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let mid_reg = read_reg(state)?;

            let src = state.get_reg(src_reg);
            let mid = state.get_reg(mid_reg).as_i64() as u64;

            if src.is_fat_ref() {
                let fat_ref = src.as_fat_ref();
                let len = fat_ref.len();
                if mid <= len {
                    let element_size = std::mem::size_of::<Value>();

                    // Left slice: [0, mid)
                    let left_ref = crate::value::FatRef::new(
                        fat_ref.ptr(),
                        fat_ref.generation(),
                        fat_ref.epoch(),
                        fat_ref.capabilities(),
                        mid,
                    );

                    // Right slice: [mid, len)
                    let right_ptr = unsafe { (fat_ref.ptr() as *const u8).add(mid as usize * element_size) };
                    let right_ref = crate::value::FatRef::new(
                        right_ptr as *mut u8,
                        fat_ref.generation(),
                        fat_ref.epoch(),
                        fat_ref.capabilities(),
                        len - mid,
                    );

                    state.set_reg(dst1, Value::from_fat_ref(left_ref));
                    state.set_reg(dst2, Value::from_fat_ref(right_ref));
                } else {
                    return Err(crate::interpreter::InterpreterError::IndexOutOfBounds {
                        index: mid as i64,
                        length: len as usize,
                    });
                }
            } else {
                state.set_reg(dst1, Value::nil());
                state.set_reg(dst2, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Capability Operations (0x10-0x1F)
        // ================================================================
        Some(CbgrSubOpcode::CapAttenuate) => {
            // Attenuate capabilities (remove permissions)
            // Format: dst:reg, src:reg, cap_mask:u16
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let cap_mask = read_u16(state)? as u32;

            let src_val = state.get_reg(src_reg);

            if is_cbgr_ref(&src_val) {
                // Register-based CBGR reference: attenuate by stripping mutability if WRITE not in mask
                let can_write = (cap_mask & caps::WRITE) != 0;
                if can_write {
                    state.set_reg(dst, src_val);
                } else {
                    // Strip mutability - downgrade &mut to &
                    let attenuated = strip_cbgr_ref_mutability(src_val.as_i64());
                    state.set_reg(dst, Value::from_i64(attenuated));
                }
            } else if src_val.is_ptr() && !src_val.is_nil() {
                // Heap-based reference: attenuate by removing from mutable set if WRITE not in mask
                let ptr_addr = src_val.as_ptr::<u8>() as usize;
                let can_write = (cap_mask & caps::WRITE) != 0;
                if !can_write {
                    state.cbgr_mutable_ptrs.remove(&ptr_addr);
                }
                state.set_reg(dst, src_val);
            } else {
                state.set_reg(dst, src_val);
            }
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::CapTransfer) => {
            // Transfer ownership (move semantics)
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src = state.get_reg(src_reg);

            // Transfer ownership: copy to dst, invalidate src
            state.set_reg(dst, src);

            if is_cbgr_ref(&src) {
                // For register-based refs, bump the generation to invalidate source
                let (abs_index, _) = decode_cbgr_ref(src.as_i64());
                state.registers.bump_generation(abs_index);
            } else if src.is_ptr() && !src.is_nil() {
                // For heap-based refs, remove from mutable set
                let ptr_addr = src.as_ptr::<u8>() as usize;
                state.cbgr_mutable_ptrs.remove(&ptr_addr);
            }
            state.set_reg(src_reg, Value::nil());
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::CapCheck) => {
            // Check if reference has specific capability
            // Format: dst:reg, src:reg, cap:u8
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let cap = read_u8(state)? as u32;

            let src_val = state.get_reg(src_reg);

            let has_cap = if is_cbgr_ref(&src_val) {
                let is_mut = is_cbgr_ref_mutable(src_val.as_i64());
                check_capabilities_for_mutability(cap, is_mut)
            } else if src_val.is_ptr() && !src_val.is_nil() {
                let ptr_addr = src_val.as_ptr::<u8>() as usize;
                let is_mut = state.cbgr_mutable_ptrs.contains(&ptr_addr);
                check_capabilities_for_mutability(cap, is_mut)
            } else if src_val.is_nil() {
                false
            } else {
                true // Non-reference types have all capabilities
            };

            state.set_reg(dst, Value::from_bool(has_cap));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::CapGet) => {
            // Get current capability mask from reference
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src_val = state.get_reg(src_reg);

            let cap_mask = if is_cbgr_ref(&src_val) {
                let is_mut = is_cbgr_ref_mutable(src_val.as_i64());
                if is_mut {
                    // Mutable ref: READ | WRITE | MUTABLE | DELEGATE | REVOKE
                    caps::OWNER
                } else {
                    // Immutable ref: READ | BORROWED | DELEGATE
                    caps::READ | caps::BORROWED | caps::DELEGATE
                }
            } else if src_val.is_ptr() && !src_val.is_nil() {
                let ptr_addr = src_val.as_ptr::<u8>() as usize;
                let is_mut = state.cbgr_mutable_ptrs.contains(&ptr_addr);
                if is_mut {
                    caps::OWNER
                } else {
                    caps::READ | caps::BORROWED | caps::DELEGATE
                }
            } else if src_val.is_nil() {
                0 // Null has no capabilities
            } else {
                caps::ALL // Non-reference types have all capabilities
            };

            state.set_reg(dst, Value::from_i64(cap_mask as i64));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::MakeShared) => {
            // Create shared reference (strip mutability, add BORROWED)
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src = state.get_reg(src_reg);

            if is_cbgr_ref(&src) {
                // Strip mutability to create shared reference
                let shared = strip_cbgr_ref_mutability(src.as_i64());
                state.set_reg(dst, Value::from_i64(shared));
            } else if src.is_ptr() && !src.is_nil() {
                // Remove from mutable set to create shared reference
                let ptr_addr = src.as_ptr::<u8>() as usize;
                state.cbgr_mutable_ptrs.remove(&ptr_addr);
                state.set_reg(dst, src);
            } else {
                state.set_reg(dst, src);
            }
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::MakeExclusive) => {
            // Create exclusive reference (ensure no aliasing, add WRITE)
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src = state.get_reg(src_reg);

            if is_cbgr_ref(&src) {
                // For register-based refs, create mutable version
                let (abs_index, generation) = decode_cbgr_ref(src.as_i64());
                let exclusive = encode_cbgr_ref_mut(abs_index, generation);
                state.set_reg(dst, Value::from_i64(exclusive));
            } else if src.is_ptr() && !src.is_nil() {
                // Add to mutable set to mark as exclusive
                let ptr_addr = src.as_ptr::<u8>() as usize;
                state.cbgr_mutable_ptrs.insert(ptr_addr);
                state.set_reg(dst, src);
            } else {
                state.set_reg(dst, src);
            }
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Generation and Epoch Operations (0x20-0x2F)
        // ================================================================
        Some(CbgrSubOpcode::GetGeneration) => {
            // Get generation counter from reference
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src_val = state.get_reg(src_reg);

            let generation = if is_cbgr_ref(&src_val) {
                // Register-based ref: extract generation from encoded value
                let (_, ref_gen) = decode_cbgr_ref(src_val.as_i64());
                ref_gen as i64
            } else if src_val.is_ptr() && !src_val.is_nil() {
                // Heap-based ref: read generation from AllocationHeader
                let ptr_addr = src_val.as_ptr::<u8>() as usize;
                let header_addr = ptr_addr.wrapping_sub(32); // 32-byte AllocationHeader
                // Read generation at offset 8 in the header
                let gen_ptr = (header_addr + 8) as *const u32;
                unsafe { *gen_ptr as i64 }
            } else {
                0 // Null or non-reference: no generation
            };

            state.set_reg(dst, Value::from_i64(generation));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::GetEpoch) => {
            // Get epoch from reference
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src_val = state.get_reg(src_reg);

            let epoch = if is_cbgr_ref(&src_val) {
                // Register-based ref: get epoch from register file
                let (abs_index, _) = decode_cbgr_ref(src_val.as_i64());
                state.registers.get_epoch(abs_index) as i64
            } else if src_val.is_ptr() && !src_val.is_nil() {
                // Heap-based ref: read epoch from AllocationHeader (offset 12, 2 bytes)
                let ptr_addr = src_val.as_ptr::<u8>() as usize;
                let header_addr = ptr_addr.wrapping_sub(32);
                let epoch_ptr = (header_addr + 12) as *const u16;
                unsafe { *epoch_ptr as i64 }
            } else {
                state.cbgr_epoch as i64 // For non-refs, return current epoch
            };

            state.set_reg(dst, Value::from_i64(epoch));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::ValidateEpoch) => {
            // Validate reference against current epoch using window comparison
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src_val = state.get_reg(src_reg);

            let is_valid = if is_cbgr_ref(&src_val) {
                // Register-based ref: check generation matches and epoch is within window
                let (abs_index, ref_gen) = decode_cbgr_ref(src_val.as_i64());
                let current_gen = state.registers.get_generation(abs_index);
                let ref_epoch = state.registers.get_epoch(abs_index);
                let global_epoch = state.registers.global_epoch();
                ref_gen == current_gen && validate_epoch_window(ref_epoch, global_epoch, EPOCH_WINDOW_SIZE)
            } else if src_val.is_ptr() && !src_val.is_nil() {
                // Heap-based ref: validate epoch using window comparison
                let ptr_addr = src_val.as_ptr::<u8>() as usize;
                let header_addr = ptr_addr.wrapping_sub(32);
                let epoch_ptr = (header_addr + 12) as *const u16;
                let ref_epoch = unsafe { *epoch_ptr };
                validate_epoch_window(ref_epoch, state.cbgr_epoch, EPOCH_WINDOW_SIZE)
            } else if src_val.is_nil() {
                false // Null references are always invalid
            } else {
                true // Non-reference types are always valid
            };

            state.set_reg(dst, Value::from_bool(is_valid));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::AdvanceEpoch) => {
            // Advance thread-local epoch
            // Format: (no operands)
            state.cbgr_epoch = state.cbgr_epoch.wrapping_add(1);
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::CurrentEpoch) => {
            // Get current thread-local epoch
            // Format: dst:reg
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(state.cbgr_epoch as i64));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::PinToEpoch) => {
            // Pin reference to current epoch
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src = state.get_reg(src_reg);
            state.set_reg(dst, src);
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Reference Conversion (0x30-0x3F)
        // ================================================================
        Some(CbgrSubOpcode::ThinToFat) => {
            // Convert thin reference to fat reference (with metadata)
            // Format: dst:reg, src:reg, metadata:reg
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let _metadata_reg = read_reg(state)?;

            let src = state.get_reg(src_reg);
            state.set_reg(dst, src);
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::FatToThin) => {
            // Convert fat reference to thin reference (discard metadata)
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src = state.get_reg(src_reg);
            state.set_reg(dst, src);
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::ToRawPtr) => {
            // Create raw pointer from reference (unchecked)
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src = state.get_reg(src_reg);
            state.set_reg(dst, src);
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::FromRawPtr) => {
            // Create reference from raw pointer (unsafe)
            // Format: dst:reg, ptr:reg, generation:reg, caps:reg
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let _generation_reg = read_reg(state)?;
            let _caps_reg = read_reg(state)?;

            let ptr = state.get_reg(ptr_reg);
            state.set_reg(dst, ptr);
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::Reborrow) => {
            // Reborrow reference with same capabilities
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src = state.get_reg(src_reg);
            state.set_reg(dst, src);
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Debug and Introspection (0x40-0x4F)
        // ================================================================
        Some(CbgrSubOpcode::DebugRef) => {
            // Dump reference metadata for debugging
            // Format: src:reg
            let src_reg = read_reg(state)?;

            let src = state.get_reg(src_reg);
            // In debug builds, could print reference info
            let _ = src; // Suppress unused warning
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::GetTier) => {
            // Get reference tier (0=managed, 1=checked, 2=unsafe)
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let _src_reg = read_reg(state)?;

            // Interpreter uses tier 0 (managed)
            state.set_reg(dst, Value::from_i64(0));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::IsValid) => {
            // Check if reference is valid (not dangling)
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src = state.get_reg(src_reg);
            let is_valid = if src.is_ptr() && !src.is_nil() {
                // Check CBGR freed flag for data pointers
                // Layout: [size:4][align:4][generation:4][epoch:2][caps:2][type_id:4][flags:4][reserved:8]
                let data_ptr = src.as_ptr::<u8>() as usize;
                let header_addr = data_ptr.wrapping_sub(32); // 32-byte AllocationHeader
                if state.cbgr_allocations.contains(&header_addr) {
                    let flags = unsafe { *((header_addr + 20) as *const u32) }; // flags at offset 20
                    flags & 1 == 0 // Valid if not freed
                } else {
                    true
                }
            } else if is_cbgr_ref(&src) {
                // Register-based reference: check generation
                let (abs_index, generation) = decode_cbgr_ref(src.as_i64());
                if generation == CBGR_NO_CHECK_GENERATION {
                    true
                } else {
                    let current_gen = state.registers.get_generation(abs_index);
                    generation == current_gen
                }
            } else {
                false
            };
            state.set_reg(dst, Value::from_bool(is_valid));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::RefCount) => {
            // Get reference count (for shared references)
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let _src_reg = read_reg(state)?;

            // Simplified: return 1 (single owner)
            state.set_reg(dst, Value::from_i64(1));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // CBGR Management (0x50-0x5F)
        // ================================================================
        Some(CbgrSubOpcode::NewGeneration) => {
            // Create new generation counter
            // Format: dst:reg
            let dst = read_reg(state)?;
            // Allocate new generation ID (simple counter-based)
            let new_gen = state.cbgr_epoch.wrapping_add(1) as i64;
            state.set_reg(dst, Value::from_i64(new_gen));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::Invalidate) => {
            // Invalidate a register slot by bumping its CBGR generation.
            // Format: src:reg
            // After this, any references captured with the old generation will
            // fail validation on dereference (use-after-free detection).
            let src_reg = read_reg(state)?;
            let abs_index = (state.reg_base() + src_reg.0 as u32) as u32;
            state.registers.bump_generation(abs_index);
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::GetEpochCaps) => {
            // Get epoch and capabilities combined from a CBGR reference
            // Format: dst:reg, src:reg
            // If src is a CBGR ref (struct with [ptr, epoch_caps]), extract epoch_caps
            // Otherwise return current epoch with full capabilities
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let src_val = state.get_reg(src_reg);

            let epoch_caps = if src_val.is_ptr() {
                let ref_ptr = src_val.as_ptr::<i64>();
                if !ref_ptr.is_null() {
                    // CBGR ref layout: [ptr: i64, epoch_caps: i64]
                    // Try to read epoch_caps from offset 1
                    unsafe { *ref_ptr.add(1) }
                } else {
                    ((state.cbgr_epoch as i64) << 32) | 0xFF
                }
            } else if src_val.is_int() {
                // May already be a packed epoch_caps value
                src_val.as_i64()
            } else {
                ((state.cbgr_epoch as i64) << 32) | 0xFF
            };
            state.set_reg(dst, Value::from_i64(epoch_caps));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::BypassBegin) => {
            // Begin CBGR bypass mode
            // Format: (no operands)
            state.cbgr_bypass_depth += 1;
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::BypassEnd) => {
            // End CBGR bypass mode
            // Format: (no operands)
            if state.cbgr_bypass_depth > 0 {
                state.cbgr_bypass_depth -= 1;
            }
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::GetStats) => {
            // Get CBGR statistics as packed i64
            // Format: dst:reg
            // Packing: [allocations:u16 | cbgr_alloc_count:u16 | epoch:u16 | validation_count:u16]
            let dst = read_reg(state)?;
            let allocations = (state.stats.allocations as u16) as i64;
            let cbgr_allocs = (state.cbgr_allocations.len() as u16) as i64;
            let epoch = (state.cbgr_epoch as u16) as i64;
            let validation = (state.cbgr_validation_count as u16) as i64;
            let packed = (allocations << 48) | (cbgr_allocs << 32) | (epoch << 16) | validation;
            state.set_reg(dst, Value::from_i64(packed));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Unimplemented sub-opcodes
        // ================================================================
        None => {
            Err(InterpreterError::NotImplemented {
                feature: "cbgr_extended sub-opcode",
                opcode: Some(Opcode::CbgrExtended),
            })
        }
    }
}

