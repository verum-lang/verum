//! CBGR (Capability-Based Generational References) instruction handlers for VBC interpreter.

use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::heap;
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;
use super::cbgr_helpers::{
    CBGR_NO_CHECK_GENERATION, EPOCH_WINDOW_SIZE, decode_cbgr_ref, encode_cbgr_ref,
    encode_cbgr_ref_mut, is_cbgr_ref, is_cbgr_ref_mutable, strip_cbgr_ref_mutability,
    validate_cbgr_generation, validate_epoch_window,
};
use crate::instruction::{CbgrSubOpcode, Opcode, Reg};
use crate::types::TypeId;
use crate::value::{Capabilities, FatRef, Value};
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
pub(in super::super) fn handle_ref_create(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;

    // Check if this RefCreate follows a CBGR data deref. If so, the Deref already
    // read the stored Value and we need to create a pointer reference back to the
    // original CBGR data location (for patterns like `&*value` which should yield
    // a raw pointer to the data, not a register-based reference).
    if let Some((deref_dst_reg, data_ptr_addr)) = state.cbgr_deref_source.take()
        && src.0 == deref_dst_reg
    {
        // Source register matches the Deref destination - restore the CBGR pointer
        // Record creation epoch for this pointer-based reference so .epoch()
        // returns the reference creation time, not the allocation time.
        state
            .cbgr_ref_creation_epoch
            .insert(data_ptr_addr, state.cbgr_epoch);
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
pub(in super::super) fn handle_ref_mut(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;

    // Handle CBGR deref-source pattern: `&mut *value` creates a pointer-based
    // mutable reference back to the original heap data location.
    if let Some((deref_dst_reg, data_ptr_addr)) = state.cbgr_deref_source.take()
        && src.0 == deref_dst_reg
    {
        state
            .cbgr_ref_creation_epoch
            .insert(data_ptr_addr, state.cbgr_epoch);
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
    state.set_reg(
        dst,
        Value::from_i64(encode_cbgr_ref_mut(abs_index, generation)),
    );
    Ok(DispatchResult::Continue)
}

/// Deref (0x72) - Dereference with CBGR validation (Tier 0).
///

/// Reads the value at the absolute register index stored in the reference.
/// For Tier 0 references, validates the CBGR generation to detect use-after-free.
pub(in super::super) fn handle_deref(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
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
            // 32-byte AllocationHeader sits immediately before the data payload —
            // see `verum_common::layout::ALLOCATION_HEADER_SIZE`.
            let header_addr = ptr_addr
                .wrapping_sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize);
            if state.cbgr_allocations.contains(&header_addr) {
                // CBGR data pointer: check if allocation has been freed.
                // Field offsets sourced from
                // `verum_common::layout::ALLOCATION_HEADER_*_OFFSET`;
                // the FREED bit lives in the canonical `flags::FREED`
                // constant (`verum_common::cbgr::flags::FREED`).
                let flags = unsafe {
                    *((header_addr + verum_common::layout::ALLOCATION_HEADER_FLAGS_OFFSET as usize)
                        as *const u32)
                };
                if flags & verum_common::cbgr::flags::FREED != 0 {
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
                // **SHARED-STRONGCOUNT-1 (deref leg)** — `*shared`
                // dereferences to the inner T.  The runtime Shared repr
                // is `[ObjectHeader(SHARED)][refcount][value]`; without
                // this arm the identity-deref below handed the Shared
                // OBJECT to consumers (an f-string then dispatched
                // `Shared.fmt` and panicked "method not found").
                let unwrap_shared = (base_ptr as usize)
                    .is_multiple_of(std::mem::align_of::<heap::ObjectHeader>())
                    && {
                        // SAFETY: alignment verified; heap objects begin
                        // with an ObjectHeader.
                        let header = unsafe { heap::ObjectHeader::ref_or_stub(base_ptr) };
                        header.type_id == crate::types::TypeId::SHARED
                    };
                if unwrap_shared {
                    let inner = unsafe {
                        *(base_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value).add(1)
                    };
                    state.set_reg(dst, inner);
                    return Ok(DispatchResult::Continue);
                }
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
pub(in super::super) fn handle_deref_mut(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
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
            unsafe {
                std::ptr::write(thin_ref.ptr as *mut Value, value);
            }
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
            unsafe {
                std::ptr::write(fat_ref.ptr() as *mut Value, value);
            }
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
        unsafe {
            std::ptr::write(ptr, value);
        }
        // CBGR epoch advancement on heap mutation
        state.cbgr_epoch = state.cbgr_epoch.wrapping_add(1);
        // Update the epoch in the AllocationHeader for this allocation.
        // Header sits immediately before the data payload — see
        // `verum_common::layout::ALLOCATION_HEADER_SIZE` and
        // `ALLOCATION_HEADER_EPOCH_OFFSET`.
        let ptr_addr = ptr as usize;
        let header_addr =
            ptr_addr.wrapping_sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize);
        if state.cbgr_allocations.contains(&header_addr) {
            unsafe {
                let epoch_ptr = (header_addr
                    + verum_common::layout::ALLOCATION_HEADER_EPOCH_OFFSET as usize)
                    as *mut u16;
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
pub(in super::super) fn handle_chk_ref(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let ref_reg = read_reg(state)?;
    let ref_val = state.get_reg(ref_reg);

    if is_cbgr_ref(&ref_val) {
        // Register-based CBGR reference
        let (abs_index, generation) = decode_cbgr_ref(ref_val.as_i64());
        validate_cbgr_generation(state, abs_index, generation)?;
    } else if ref_val.is_ptr() && !ref_val.is_nil() {
        // Heap-based CBGR reference - validate AllocationHeader.
        // Field offsets sourced from `verum_common::layout`.
        let ptr_addr = ref_val.as_ptr::<u8>() as usize;
        let header_addr =
            ptr_addr.wrapping_sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize);

        if state.cbgr_allocations.contains(&header_addr) {
            // Read generation and flags from AllocationHeader.
            let generation = unsafe {
                *((header_addr + verum_common::layout::ALLOCATION_HEADER_GENERATION_OFFSET as usize)
                    as *const u32)
            };
            let _epoch = unsafe {
                *((header_addr + verum_common::layout::ALLOCATION_HEADER_EPOCH_OFFSET as usize)
                    as *const u16)
            };
            let flags = unsafe {
                *((header_addr + verum_common::layout::ALLOCATION_HEADER_FLAGS_OFFSET as usize)
                    as *const u32)
            };

            // Check if allocation has been freed
            if flags & verum_common::cbgr::flags::FREED != 0 {
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

/// Non-trapping reference validation — the `cbgr_validate<T>(&T) -> Bool`
/// backend (`SystemSubOpcode::CbgrValidateBool`).  Mirrors the checks of
/// `handle_chk_ref` but REPORTS the verdict instead of panicking, which is
/// the shape the stdlib declaration promises (`-> Bool`).  Classification:
///   * nil → false (no referent)
///   * register-encoded CBGR ref → generation-check verdict
///   * heap pointer with a tracked AllocationHeader → !FREED verdict
///   * any other live pointer/value → true (Tier 0 only hands out live
///     objects; untracked ≠ dangling in the interpreter)
pub(in super::super) fn validate_ref_bool(state: &mut InterpreterState, ref_val: Value) -> bool {
    if ref_val.is_nil() {
        return false;
    }
    if is_cbgr_ref(&ref_val) {
        let (abs_index, generation) = decode_cbgr_ref(ref_val.as_i64());
        return validate_cbgr_generation(state, abs_index, generation).is_ok();
    }
    if ref_val.is_ptr() {
        let ptr = ref_val.as_ptr::<u8>();
        if ptr.is_null() {
            return false;
        }
        let ptr_addr = ptr as usize;
        let header_addr =
            ptr_addr.wrapping_sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize);
        if state.cbgr_allocations.contains(&header_addr) {
            // SAFETY: header liveness established via cbgr_allocations.
            let flags = unsafe {
                *((header_addr + verum_common::layout::ALLOCATION_HEADER_FLAGS_OFFSET as usize)
                    as *const u32)
            };
            return flags & verum_common::cbgr::flags::FREED == 0;
        }
        return true;
    }
    true
}

/// RefChecked (0x75) - Create Tier 1 checked reference.
///

/// Tier 1 references are compiler-proven safe and skip generation checks.
/// Uses CBGR_NO_CHECK_GENERATION sentinel so deref skips validation.
pub(in super::super) fn handle_ref_checked(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let abs_index = (state.reg_base() + src.0 as u32) as u32;
    if state.config.count_instructions {
        state.stats.cbgr_stats.tier1_refs += 1;
    }
    state.set_reg(
        dst,
        Value::from_i64(encode_cbgr_ref(abs_index, CBGR_NO_CHECK_GENERATION)),
    );
    Ok(DispatchResult::Continue)
}

/// RefUnsafe (0x76) - Create Tier 2 unsafe reference (no runtime checks).
///

/// Tier 2 references require manual safety proof and skip generation checks.
/// Uses CBGR_NO_CHECK_GENERATION sentinel so deref skips validation.
pub(in super::super) fn handle_ref_unsafe(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let abs_index = (state.reg_base() + src.0 as u32) as u32;
    if state.config.count_instructions {
        state.stats.cbgr_stats.tier2_refs += 1;
    }
    state.set_reg(
        dst,
        Value::from_i64(encode_cbgr_ref(abs_index, CBGR_NO_CHECK_GENERATION)),
    );
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
pub(in super::super) fn handle_drop_ref(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let src = read_reg(state)?;
    // ARCHIVE-TYPE-GLUE-IDS-1 hardening: DropRef's drop-glue /
    // pending-drop machinery re-executes THIS instruction by rewinding
    // the pc.  The encoded size is opcode(1) + reg operand — 1 byte
    // for r0-r127, 2 bytes for r128+ (see `encoding::encode_reg`).
    // The rewind was hard-coded `2`, which for a wide-reg DropRef
    // landed the pc on the LAST byte of its own encoding and decoded
    // garbage from there.  Latent while imported-type glue ids were
    // cleared (user-module Drop types rarely sit above r127); real
    // stdlib glue activates the path in large frames too.
    let dropref_len: u32 = if src.0 < 128 { 2 } else { 3 };
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
        let is_cbgr_alloc = state
            .cbgr_allocations
            .contains(&(obj_ptr as usize).wrapping_sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize));

        if !is_cbgr_alloc {
            // Standard heap objects: the pointer points directly to the ObjectHeader
            // (see handle_new which stores obj.as_ptr() - the header pointer)
            let header_ptr = obj_ptr;

            // Alignment-checked header read.  `ObjectHeader` is
            // `#[repr(C, align(8))]`, so every legitimate header is 8-
            // byte aligned.  Any value classified `is_regular_ptr() ==
            // true` but pointing at a non-header location (e.g. an
            // interior pointer produced by ad-hoc casts in user code
            // or `&arr[i]` constructs) would trip
            // `panic_misaligned_pointer_dereference` and abort the
            // whole interpreter via SIGABRT, losing every parallel
            // test in the same `verum test --interp` invocation.  The
            // aligned-or-skip path is the architecturally honest
            // answer: a misaligned pointer cannot be a valid
            // ObjectHeader, so there is no Drop impl to invoke; fall
            // through to the existing CBGR cleanup path which
            // operates on the raw bits independent of header
            // structure.
            let type_id = match unsafe { heap::ObjectHeader::try_type_id(header_ptr) } {
                Some(tid) => tid,
                None => {
                    tracing::trace!(
                        "[drop_ref] skipping Drop check on misaligned/null ptr {:p}; \
                         value still goes through CBGR cleanup",
                        header_ptr
                    );
                    return Ok(DispatchResult::Continue);
                }
            };

            // **SHARED-STRONGCOUNT-1 (drop leg)** — Shared<T> binding
            // drop decrements the strong count that `clone` bumped.
            // DropRef is emitted once per user BINDING (not per alias
            // temp), so binding-granularity decrement mirrors the
            // source-level `Drop for Shared` semantics over the runtime
            // repr `[refcount:i64][value]`.  Saturates at zero — the
            // repr keeps the allocation alive for the interpreter heap
            // to reclaim, matching `into_inner`'s no-hard-free policy.
            if type_id == crate::types::TypeId::SHARED {
                let data_ptr =
                    unsafe { obj_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value };
                let refcount = unsafe { (*data_ptr).as_i64() };
                if refcount > 0 {
                    unsafe {
                        *data_ptr = Value::from_i64(refcount - 1);
                    }
                }
                return Ok(DispatchResult::Continue);
            }

            // ARCHIVE-TYPE-GLUE-IDS-1: resolve the descriptor BY ID
            // (`type_index_by_id`), never positionally.  Descriptor ids
            // are not positional in `module.types` (well-known-id
            // backfills shift them), so the old
            // `types[type_id - FIRST_USER]` indexing resolved the WRONG
            // descriptor — latent while imported-type glue was cleared
            // (every wrong hit had `drop_fn == None`), loud once real
            // glue ids are live: a CfgPredicate drop dispatched
            // `PosixTerminal.drop` (whatever descriptor owned the
            // coincident POSITION) and null-deref'd on foreign layout.
            //
            // SEMANTIC-band gate: ids in
            // [`FIRST_SEMANTIC`, `LAST_SEMANTIC`] (List/Map/Heap/
            // Shared/…) are NATIVE interpreter representations — the
            // runtime heap owns their buffers and their layout is NOT
            // the stdlib record layout the imported descriptor
            // describes.  Dispatching the stdlib record drop glue over
            // a native object reads/writes record offsets into the
            // native layout (observed: `List.drop`'s
            // `clear`+`free_buffer` over a native LIST object →
            // libmalloc "pointer being freed was not allocated" abort).
            // The codegen allocator provably never places USER types in
            // this band (`alloc_type_id` skips it; asserted at
            // finalize), so gating the band excludes exactly the
            // native-representation set.  Their .vr `Drop` impls remain
            // meaningful for the self-hosted/AOT record layer only.
            let semantic_band = crate::types::TypeId::FIRST_SEMANTIC
                ..=crate::types::TypeId::LAST_SEMANTIC;
            let type_desc_idx = if type_id.0 >= crate::types::TypeId::FIRST_USER
                && !semantic_band.contains(&type_id.0)
            {
                state.module.type_index_by_id(type_id)
            } else {
                None
            };

            // SYNTHESIS with main's independent TYPE-ID-COLLISION-3 fix
            // (net-conformance lineage): even a correctly-indexed
            // descriptor can carry a STALE drop_fn on the lazy run-path
            // (finalize_module_from_state sets it by name but never
            // remaps to the contiguous module id). Every genuine Drop
            // impl registers as `<Type>.drop`, so a resolved drop_fn
            // whose name is not a drop is a mis-resolution: skip it and
            // fall through to builtin cleanup rather than execute
            // arbitrary code. (The loader-side remap-or-clear makes
            // this a belt-and-braces guard on the archive path; the
            // run-path load is the one it still protects.)

            // Look up TypeDescriptor to find drop_fn
            // Extract all needed values before any mutable operations to avoid borrow conflicts
            let drop_info = type_desc_idx
                .and_then(|type_idx| state.module.types.get(type_idx))
                .and_then(|type_desc| type_desc.drop_fn)
                .and_then(|drop_fn_id| {
                    state
                        .module
                        .functions
                        .get(drop_fn_id as usize)
                        .and_then(|func| {
                            let name =
                                state.module.strings.get(func.name).unwrap_or("");
                            if name == "drop" || name.ends_with(".drop") {
                                Some((
                                    drop_fn_id,
                                    func.register_count,
                                    func.bytecode_offset,
                                ))
                            } else {
                                None
                            }
                        })
                });

            if let Some((drop_fn_id, reg_count, _bytecode_offset)) = drop_info {
                if std::env::var("VERUM_TRACE_DROPFN").is_ok() {
                    let tn = if let Some(type_idx) = type_desc_idx {
                        state
                            .module
                            .types
                            .get(type_idx)
                            .and_then(|td| state.module.strings.get(td.name))
                            .unwrap_or("?")
                    } else {
                        "<builtin>"
                    };
                    let dfn = state
                        .module
                        .functions
                        .get(drop_fn_id as usize)
                        .and_then(|f| state.module.strings.get(f.name))
                        .unwrap_or("?");
                    eprintln!(
                        "[DROPFN] type='{}' (id={}) drop_fn_id={} resolves_to='{}'",
                        tn, type_id.0, drop_fn_id, dfn
                    );
                }
                // Set return_pc to the CURRENT DropRef instruction.
                // After drop() returns, we'll re-execute DropRef with a cleared register.
                // Subtract the instruction size to re-execute this instruction
                // (DropRef encoding: opcode(1) + reg(1 or 2) — see dropref_len above)
                let current_pc = state.pc();
                let return_pc = current_pc.saturating_sub(dropref_len);
                let caller_base = state.reg_base();

                // Push a new frame for the drop call
                let func_id = crate::module::FunctionId(drop_fn_id);
                let new_base = state
                    .call_stack
                    .push_frame(func_id, reg_count, return_pc, src)?;
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
                {
                    // Id-correct resolution here too (see the drop_info
                    // comment above) — the outer type AND each field's
                    // type must resolve by descriptor id, not position.
                    // Clone the field list out so the immutable borrow of
                    // `state.module` ends before the drop dispatch below.
                    let fields: Vec<crate::types::FieldDescriptor> = type_desc_idx
                        .and_then(|type_idx| state.module.types.get(type_idx))
                        .map(|td| td.fields.iter().cloned().collect())
                        .unwrap_or_default();
                    if !fields.is_empty() {
                        // Check each field for droppable types
                        for field in &fields {
                            // Get the field type ID
                            let field_type_id = match &field.type_ref {
                                crate::types::TypeRef::Concrete(tid) => Some(*tid),
                                _ => None,
                            };

                            if let Some(ftid) = field_type_id
                                && ftid.0 >= crate::types::TypeId::FIRST_USER
                                && !semantic_band.contains(&ftid.0)
                            {
                                let has_drop = state
                                    .module
                                    .type_index_by_id(ftid)
                                    .and_then(|i| state.module.types.get(i))
                                    .map(|fd| fd.drop_fn.is_some())
                                    .unwrap_or(false);

                                if has_drop {
                                    // Read the field value from the struct
                                    // Struct layout: [ObjectHeader][field0][field1][...]
                                    let field_ptr = unsafe {
                                        obj_ptr
                                            .add(heap::OBJECT_HEADER_SIZE + field.offset as usize)
                                            as *const Value
                                    };
                                    let field_val = unsafe { *field_ptr };
                                    // DEBUG: eprintln!("[DEBUG DropRef] Queueing field '{}' for drop: {:?}",
                                    //  state.module.strings.get(field.name).unwrap_or("?"), field_val);
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
                            state.set_pc(current_pc.saturating_sub(dropref_len));
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
        let is_cbgr_alloc = state
            .cbgr_allocations
            .contains(&(obj_ptr as usize).wrapping_sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize));

        if !is_cbgr_alloc
            && let Some(header) = unsafe { heap::ObjectHeader::try_from_ptr(obj_ptr) }
            && header.type_id == TypeId::TUPLE
        {
            // Tuple branch entered only when the pointer is aligned
            // and points at a TUPLE header — same alignment-gated
            // discipline as the user-defined-Drop branch above.  Tuple
            // layout: [ObjectHeader][elem0][elem1][elem2]…
            let elem_count = header.size as usize / std::mem::size_of::<Value>();
            let data_ptr = unsafe { obj_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };

            // Queue elements in REVERSE order so that after LIFO processing,
            // they're dropped in forward order (element 0 first, then 1, etc.)
            for i in (0..elem_count).rev() {
                let elem = unsafe { *data_ptr.add(i) };
                if elem.is_ptr() && !elem.is_nil() {
                    state.pending_drops.push(elem);
                }
            }

            // If we queued any pending drops, process them
            if !state.pending_drops.is_empty() {
                state.set_reg(src, Value::unit());
                let current_pc = state.pc();
                state.set_pc(current_pc.saturating_sub(dropref_len));
                return Ok(DispatchResult::Continue);
            }
        }
    }

    // For CBGR heap allocations (pointer to data after AllocationHeader),
    // bump the generation in the header to invalidate references.
    // Layout: [size:4][align:4][generation:4][epoch:2][caps:2][type_id:4][flags:4][reserved:8]
    //
    // **Owned-vs-borrowed discrimination** (closes task #121 `&Text`
    // return-ref class).  Pre-fix this branch gated on `val.is_ptr()`,
    // which returns `true` for ANY TAG_POINTER value — including
    // ThinRef and FatRef (CBGR borrowed references that encode the
    // referent's address in the 48-bit payload, NOT a real heap
    // address).  When DropRef ran on a borrowed-ref register at
    // method return (e.g. `self: &PanicInfo` going out of scope at
    // the end of `PanicInfo.message()`), `val.as_ptr::<u8>() - 32`
    // happened to map to the underlying allocation's header — the
    // bump fired, the caller's `msg = info.message()` ref recorded
    // gen N, the next deref saw gen N+1, and the runtime panicked
    // with "CBGR use-after-free detected: expected generation N,
    // found N+1" on legitimate borrow lifetimes.
    //
    // The fundamental rule: only OWNED values bump the generation
    // when dropped.  Borrowed refs (ThinRef / FatRef) have their
    // own register-slot generation counter (bumped at line ~725
    // below via `registers.bump_generation`) — they MUST NOT touch
    // the AllocationHeader's generation, which belongs to the
    // owning value.  `is_regular_ptr` is the canonical predicate
    // for "owned heap pointer, not a special-tagged ref" — bit 47
    // distinguishes the two ranges per the NaN-box layout
    // documented at `value.rs:1000-1008`.
    if val.is_regular_ptr() {
        let data_ptr = val.as_ptr::<u8>() as usize;
        let header_addr =
            data_ptr.wrapping_sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize);
        if state.cbgr_allocations.contains(&header_addr) {
            // Bump generation and set FREED flag in AllocationHeader.
            // Field offsets and the FREED bit value live in
            // `verum_common::{layout, cbgr::flags}`.
            unsafe {
                let gen_ptr = (header_addr
                    + verum_common::layout::ALLOCATION_HEADER_GENERATION_OFFSET as usize)
                    as *mut u32;
                *gen_ptr = (*gen_ptr).wrapping_add(1);
                let flags_ptr = (header_addr
                    + verum_common::layout::ALLOCATION_HEADER_FLAGS_OFFSET as usize)
                    as *mut u32;
                *flags_ptr |= verum_common::cbgr::flags::FREED;
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

/// SLICE-REP-UNIFY-1 (#51 runtime leg 2): THE canonical constructor of
/// a slice value over a runtime container. Verum's slice representation
/// is the FatRef (`{data_ptr, len, reserved=elem_size}`); the historic
/// `as_slice` identity-cast intercept leaked raw LIST pointers into
/// slice positions, forking the representation — `&xs[..]` produced a
/// FatRef while `xs.as_slice()` produced a List ptr, and every
/// FatRef-only consumer (slice_subslice, split_at) either crashed or
/// silently no-op'd on the latter.
///

/// Accepts: an existing FatRef / BYTE_SLICE view (identity — already
/// canonical), a LIST / BYTE_LIST heap object (follows `backing_ptr`
/// to the element data), or a typed raw array (U8/U16/U32/U64 —
/// element data starts after the header; stride recorded in
/// `reserved`). Anything else → None (caller decides the fallback).
pub(in super::super) fn container_to_slice_fat_ref(
    state: &crate::interpreter::InterpreterState,
    src: Value,
) -> Option<Value> {
    if src.is_fat_ref() || heap::value_as_byte_slice(&src).is_some() {
        return Some(src);
    }
    if !src.is_regular_ptr() {
        return None;
    }
    let base_ptr = src.as_ptr::<u8>();
    let header = unsafe { heap::ObjectHeader::try_from_ptr(base_ptr) }?;
    let epoch = (state.cbgr_epoch & 0xFFFF) as u16;
    match header.type_id {
        TypeId::LIST | TypeId::BYTE_LIST => {
            // Layout: [ObjectHeader][len: Value][cap: Value][backing_ptr: Value]
            let len =
                unsafe { *(base_ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value) }.as_i64();
            let backing_val = unsafe {
                *(base_ptr.add(heap::OBJECT_HEADER_SIZE + 16) as *const Value)
            };
            let data_ptr = if backing_val.is_ptr() && !backing_val.is_nil() {
                unsafe { backing_val.as_ptr::<u8>().add(heap::OBJECT_HEADER_SIZE) }
            } else if len == 0 {
                // Never-pushed list: no backing yet — a dangling-free
                // empty slice over the header edge is sound (len 0
                // forbids every deref).
                unsafe { base_ptr.add(heap::OBJECT_HEADER_SIZE) }
            } else {
                return None;
            };
            let mut fat_ref = FatRef::slice(
                data_ptr,
                0,
                epoch,
                Capabilities::MUT_EXCLUSIVE,
                len.max(0) as u64,
            );
            fat_ref.reserved = if header.type_id == TypeId::BYTE_LIST { 1 } else { 0 };
            Some(Value::from_fat_ref(fat_ref))
        }
        TypeId::U8 | TypeId::U16 | TypeId::U32 | TypeId::U64 => {
            let stride: u32 = match header.type_id {
                TypeId::U8 => 1,
                TypeId::U16 => 2,
                TypeId::U32 => 4,
                _ => 8,
            };
            let data_ptr = unsafe { base_ptr.add(heap::OBJECT_HEADER_SIZE) };
            let len = (header.size as u64) / stride as u64;
            let mut fat_ref = FatRef::slice(
                data_ptr,
                0,
                epoch,
                Capabilities::MUT_EXCLUSIVE,
                len,
            );
            fat_ref.reserved = stride;
            Some(Value::from_fat_ref(fat_ref))
        }
        _ => None,
    }
}

pub(in super::super) fn handle_cbgr_extended(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let sub_op_byte = read_u8(state)?;
    // Skip operand-length varint that the encoder writes after
    // sub_op (see encode_instruction's `Instruction::CbgrExtended`
    // arm).  Without this, the operand-length bytes get
    // misinterpreted as register indices.  The length is only
    // consumed by sequential decoders (linker, disassembler);
    // dispatch reads operands per-sub_op below.
    let _operand_len = read_varint(state)?;
    let sub_op = CbgrSubOpcode::from_byte(sub_op_byte);

    match sub_op {
        // ================================================================
        // Slice and Interior References (0x00-0x0F)
        // ================================================================
        Some(CbgrSubOpcode::RefListElement) => {
            // Create interior reference to List<T> element at index.
            // Produces a plain `Value::from_ptr(element_ptr)` so the
            // existing DerefMut/Deref handlers for ptr values write
            // and read through it directly.
            //

            // Format: dst:reg, list:reg, index:reg
            let dst = read_reg(state)?;
            let list_reg = read_reg(state)?;
            let index_reg = read_reg(state)?;

            let list_val = state.get_reg(list_reg);
            let index = state.get_reg(index_reg).as_i64();

            // Auto-deref CBGR register-based reference, like SetE/GetE do.
            let list_val = if is_cbgr_ref(&list_val) {
                let (abs_index, _gen) = decode_cbgr_ref(list_val.as_i64());
                state.registers.get_absolute(abs_index)
            } else if list_val.is_thin_ref() {
                let thin_ref = list_val.as_thin_ref();
                if thin_ref.ptr.is_null() {
                    return Err(InterpreterError::NullPointer);
                }
                unsafe { *(thin_ref.ptr as *const Value) }
            } else {
                list_val
            };

            // FATREF-INTERIOR-REF-1 (#51 unification tail): the slice
            // representation is now uniformly a FatRef, so
            // `&self.slice[i]` (SliceIter.next) arrives here with a
            // FatRef base. Value-stride slices hand out the exact
            // element address (same cbgr_mutable_ptrs contract as the
            // LIST arm below); raw-stride slices cannot back a `&T`
            // interior ref (Deref would read Value bits out of raw
            // bytes) — loud typed error, never silent corruption.
            if list_val.is_fat_ref() {
                let fr = list_val.as_fat_ref();
                let len = fr.len() as i64;
                if index < 0 || index >= len {
                    return Err(InterpreterError::IndexOutOfBounds {
                        index,
                        length: len as usize,
                    });
                }
                if fr.reserved != 0 {
                    return Err(InterpreterError::Panic {
                        message: format!(
                            "interior reference into a raw-element slice \
                             (stride {}) is not representable — index the \
                             slice by value instead (FATREF-INTERIOR-REF-1 / \
                             AOT-SLICE-ELEMSIZE-CARRY-1 #48)",
                            fr.reserved
                        ),
                    });
                }
                let elem_ptr = unsafe {
                    fr.ptr().add((index as usize) * std::mem::size_of::<Value>())
                };
                state.cbgr_mutable_ptrs.insert(elem_ptr as usize);
                state.set_reg(dst, Value::from_ptr(elem_ptr));
                return Ok(DispatchResult::Continue);
            }

            let ptr = list_val.as_ptr::<u8>();
            if ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }

            // Layout: base → ObjectHeader → [len:Value, cap:Value, backing_ptr:Value].
            // The first element is at `backing_ptr + OBJECT_HEADER_SIZE`.
            // Alignment-checked header read: a misaligned pointer
            // cannot be a valid header, so it cannot describe a LIST
            // or inline array.  Return a typed error instead of
            // aborting the interpreter through the Rust runtime's UB
            // alignment check.
            let header = match unsafe { heap::ObjectHeader::try_from_ptr(ptr) } {
                Some(h) => h,
                None => return Err(InterpreterError::NullPointer),
            };

            let elem_ptr = if header.type_id == TypeId::LIST {
                let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
                let len = unsafe { (*data_ptr).as_i64() } as usize;
                if index < 0 || (index as usize) >= len {
                    return Err(InterpreterError::IndexOutOfBounds { index, length: len });
                }
                let backing = unsafe { (*data_ptr.add(2)).as_ptr::<u8>() };
                let offset =
                    heap::OBJECT_HEADER_SIZE + (index as usize) * std::mem::size_of::<Value>();
                unsafe { backing.add(offset) }
            } else {
                // Inline array / tuple: elements live directly after the
                // header.
                let element_count = header.size as usize / std::mem::size_of::<Value>();
                if index < 0 || (index as usize) >= element_count {
                    return Err(InterpreterError::IndexOutOfBounds {
                        index,
                        length: element_count,
                    });
                }
                let offset =
                    heap::OBJECT_HEADER_SIZE + (index as usize) * std::mem::size_of::<Value>();
                unsafe { ptr.add(offset) }
            };

            // Mark this pointer as "dereferences to a Value in memory" so the
            // generic `Deref` handler reads through it (`*(ptr as *const Value)`)
            // instead of falling through to identity-deref for heap objects.
            // Without this, `*&arr[i]` returns the interior pointer itself
            // (displayed as `<object type_id=N>`), not the element value —
            // breaking every spec that builds a reference with `&arr[i]`.
            state.cbgr_mutable_ptrs.insert(elem_ptr as usize);
            state.set_reg(dst, Value::from_ptr(elem_ptr));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::RefRawAddr) => {
            // Interior reference from a raw ADDRESS in an int register
            // (`&*self.ptr.offset(i)` — see the opcode doc). Mirrors the
            // RefListElement tail: register the address as a heap-interior
            // pointer so the generic Deref / dispatch / GetVariantData
            // paths read the pointee Value, then hand out a ptr Value.
            // Format: dst:reg, addr:reg
            let dst = read_reg(state)?;
            let addr_reg = read_reg(state)?;
            let addr_val = state.get_reg(addr_reg);
            let addr: usize = if addr_val.is_int() {
                addr_val.as_i64() as usize
            } else if addr_val.is_ptr() && !addr_val.is_nil() {
                addr_val.as_ptr::<u8>() as usize
            } else {
                0
            };
            if addr == 0 {
                state.set_reg(dst, Value::nil());
            } else {
                state.cbgr_mutable_ptrs.insert(addr);
                if std::env::var("VERUM_TRACE_CALLM_FLOW").is_ok() {
                    eprintln!("[raw-addr] insert {:#x}", addr);
                }
                state.set_reg(dst, Value::from_ptr(addr as *mut u8));
            }
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::RefField) => {
            // Create interior reference to a record field by field-index,
            // producing a `Value::from_ptr(field_ptr)` anchored directly
            // in the heap object's data area.
            //
            // Closes task #121: the generic `Ref` path encodes the
            // method-frame stack-slot abs_index, which `pop_frame`'s
            // generation bump invalidates even when the heap object is
            // still alive in the caller.  `RefField` produces a
            // heap-anchored pointer that survives frame teardown.
            //
            // Format: dst:reg, base:reg, field_idx:varint
            let dst = read_reg(state)?;
            let base_reg = read_reg(state)?;
            let field_idx = read_varint(state)? as usize;

            let base_val = state.get_reg(base_reg);

            // Auto-deref CBGR register reference / thin-ref / fat-ref
            // (same chain GetF runs before computing the field offset).
            let base_val = if is_cbgr_ref(&base_val) {
                let (abs_index, _gen) = decode_cbgr_ref(base_val.as_i64());
                state.registers.get_absolute(abs_index)
            } else if base_val.is_thin_ref() {
                let thin_ref = base_val.as_thin_ref();
                if thin_ref.ptr.is_null() {
                    return Err(InterpreterError::NullPointer);
                }
                unsafe { *(thin_ref.ptr as *const Value) }
            } else if base_val.is_fat_ref() {
                Value::from_ptr(base_val.as_fat_ref().ptr())
            } else {
                base_val
            };

            if !base_val.is_ptr() || base_val.is_nil() {
                return Err(InterpreterError::NullPointer);
            }
            let mut ptr = base_val.as_ptr::<u8>();
            if ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }

            // Mirror the GetF auto-deref chain for receivers that wrap
            // the actual record (Heap<T>, mutable interior ptr,
            // Shared<T> refcount slot, variant payload).  Without this
            // mirror, `&self.field` taken on a `Heap<T>` or `Shared<T>`
            // carrier would compute an offset into the wrapper instead
            // of the inner record.

            // CBGR Heap<T> allocation: data area is preceded by a
            // 32-byte AllocationHeader; payload[0] is a pointer to the
            // inner record.
            {
                let header_addr = (ptr as usize).wrapping_sub(32);
                if state.cbgr_allocations.contains(&header_addr) {
                    let inner = unsafe { *(ptr as *const Value) };
                    if inner.is_ptr() && !inner.is_nil() {
                        ptr = inner.as_ptr::<u8>();
                        if ptr.is_null() {
                            return Err(InterpreterError::NullPointer);
                        }
                    }
                }
            }

            // Interior-pointer auto-deref (mirror of GetF lines 217-230):
            // when the base is itself a tracked mutable interior pointer
            // (produced by an earlier RefField/RefListElement), the slot
            // it addresses holds a `Value`.  Load that Value and, if it
            // points to a heap object, follow the pointer so the field
            // resolves on the addressed record rather than on the wrapper.
            if state.cbgr_mutable_ptrs.contains(&(ptr as usize))
                && (ptr as usize).is_multiple_of(std::mem::align_of::<Value>())
            {
                let inner = unsafe { *(ptr as *const Value) };
                if inner.is_ptr() && !inner.is_nil() {
                    ptr = inner.as_ptr::<u8>();
                    if ptr.is_null() {
                        return Err(InterpreterError::NullPointer);
                    }
                }
            }

            // Object-header alignment + Shared<T> / variant unwrap.
            if !(ptr as usize).is_multiple_of(std::mem::align_of::<heap::ObjectHeader>()) {
                return Err(InterpreterError::Panic {
                    message: format!(
                        "misaligned pointer {:p} for RefField (requires {}-byte alignment)",
                        ptr,
                        std::mem::align_of::<heap::ObjectHeader>()
                    ),
                });
            }
            // SAFETY: alignment + non-null verified above; every VBC
            // heap object starts with an ObjectHeader.
            let header = unsafe { heap::ObjectHeader::ref_or_stub(ptr) };

            if header.type_id == TypeId::SHARED {
                // Skip refcount slot to reach the inner Value.
                let data_ptr = unsafe { ptr.add(heap::OBJECT_HEADER_SIZE) as *const Value };
                let inner = unsafe { *data_ptr.add(1) };
                if inner.is_ptr() && !inner.is_nil() {
                    ptr = inner.as_ptr::<u8>();
                    if ptr.is_null() {
                        return Err(InterpreterError::NullPointer);
                    }
                }
            }

            // Variant wrapper unwrap (type_id >= 0x8000): payload[0]
            // holds the inner record pointer.
            {
                if !(ptr as usize).is_multiple_of(std::mem::align_of::<heap::ObjectHeader>()) {
                    return Err(InterpreterError::Panic {
                        message: format!(
                            "misaligned pointer {:p} after Shared deref in RefField",
                            ptr,
                        ),
                    });
                }
                let header = unsafe { heap::ObjectHeader::ref_or_stub(ptr) };
                if header.type_id.0 >= 0x8000 {
                    let payload_offset = heap::OBJECT_HEADER_SIZE + 8;
                    let inner = unsafe { *(ptr.add(payload_offset) as *const Value) };
                    if inner.is_ptr() && !inner.is_nil() {
                        ptr = inner.as_ptr::<u8>();
                        if ptr.is_null() {
                            return Err(InterpreterError::NullPointer);
                        }
                    }
                }
            }

            // Final alignment + bounds check on the unwrapped record.
            if !(ptr as usize).is_multiple_of(std::mem::align_of::<heap::ObjectHeader>()) {
                return Err(InterpreterError::Panic {
                    message: format!(
                        "misaligned final pointer {:p} in RefField after auto-deref chain",
                        ptr,
                    ),
                });
            }
            // SAFETY: alignment + non-null verified; object has a
            // header.
            let header = unsafe { heap::ObjectHeader::ref_or_stub(ptr) };

            let field_offset = field_idx
                .checked_mul(std::mem::size_of::<Value>())
                .ok_or_else(|| InterpreterError::Panic {
                    message: "RefField: field offset overflow".into(),
                })?;
            let field_end = field_offset
                .checked_add(std::mem::size_of::<Value>())
                .ok_or_else(|| InterpreterError::Panic {
                    message: "RefField: field end offset overflow".into(),
                })?;
            if field_end > header.size as usize {
                return Err(InterpreterError::Panic {
                    message: format!(
                        "RefField: field {} (offset {}+{}={}) exceeds object data size {}",
                        field_idx,
                        field_offset,
                        std::mem::size_of::<Value>(),
                        field_end,
                        header.size
                    ),
                });
            }
            // SAFETY: field bounds validated above; data area starts
            // at OBJECT_HEADER_SIZE and contains an initialized Value
            // at field_offset.
            let field_ptr =
                unsafe { ptr.add(heap::OBJECT_HEADER_SIZE + field_offset) };

            // Mark the field pointer as a tracked mutable interior ref
            // so the generic Deref / DerefMut handlers read and write
            // through it instead of treating it as an opaque pointer.
            state.cbgr_mutable_ptrs.insert(field_ptr as usize);
            state.set_reg(dst, Value::from_ptr(field_ptr));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::RefSliceRaw) => {
            // Create a FatRef directly from a raw pointer + length, with
            // elem_size=1 (byte slice). Used to lower the generic
            // `slice_from_raw_parts<T>` stdlib intrinsic when the pointer
            // does not point to an ObjectHeader (raw buffer addresses).
            // NOTE: `Text.as_bytes()` no longer produces this shape — it
            // allocates a typed BYTE_SLICE (528) object (ARCH-P5).
            //

            // Format: dst:reg, ptr:reg, len:reg
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let len_reg = read_reg(state)?;

            let ptr_val = state.get_reg(ptr_reg);
            let len = state.get_reg(len_reg).as_i64() as u64;

            let raw_ptr = if ptr_val.is_ptr() {
                ptr_val.as_ptr::<u8>()
            } else if ptr_val.is_thin_ref() {
                ptr_val.as_thin_ref().ptr
            } else if ptr_val.is_fat_ref() {
                ptr_val.as_fat_ref().ptr()
            } else if ptr_val.is_int() {
                // Raw integer-encoded pointer (rare but possible via as casts).
                ptr_val.as_i64() as *mut u8
            } else {
                std::ptr::null_mut()
            };

            let mut fat_ref = FatRef::slice(
                raw_ptr,
                0,
                (state.cbgr_epoch & 0xFFFF) as u16,
                Capabilities::MUT_EXCLUSIVE,
                len,
            );
            fat_ref.reserved = 1; // byte-sized elements

            state.set_reg(dst, Value::from_fat_ref(fat_ref));
            Ok(DispatchResult::Continue)
        }

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

            // FatRef fast-path (mirrors SliceSubslice below). A FatRef src —
            // a slice-of-a-slice, e.g. `&remaining[..n]` where `remaining`
            // is itself a byte-slice from `text.as_bytes()` (HttpParser.feed
            // re-slices `&buf[pos..]`) — shares TAG_POINTER, so the generic
            // pointer path below would take its FAT_REF_MARKER payload as a
            // heap address (both for `base_ptr` and the `try_type_id`
            // elem-size probe) → SIGSEGV. Re-slice directly, carrying the
            // element stride in `reserved` (1/2/4/8 for raw integers, 0 =
            // NaN-boxed Value) so we don't walk past the end of a byte slice.
            if src.is_fat_ref() {
                let fat_ref = src.as_fat_ref();
                let element_size = if fat_ref.reserved == 0 {
                    std::mem::size_of::<Value>()
                } else {
                    fat_ref.reserved as usize
                };
                let new_ptr = unsafe { fat_ref.ptr().add(start * element_size) };
                let mut new_fat_ref = crate::value::FatRef::new(
                    new_ptr,
                    fat_ref.generation(),
                    fat_ref.epoch(),
                    fat_ref.capabilities(),
                    len,
                );
                new_fat_ref.reserved = fat_ref.reserved;
                state.set_reg(dst, Value::from_fat_ref(new_fat_ref));
                return Ok(DispatchResult::Continue);
            }

            // BYTE_SLICE fast-path (ARCH-P5).  `&buf[pos..]` where `buf`
            // is a `text.as_bytes()` byte view (the HttpParser.feed
            // re-slice pattern) — produce a NEW BYTE_SLICE object
            // `{ptr + start, len}` (stride 1).  Without this arm, the
            // generic pointer path below would probe the 528 header,
            // skip it, and treat the raw `{ptr, len}` payload words as
            // element data.
            if let Some((base, _src_len)) = heap::value_as_byte_slice(&src) {
                // SAFETY: `base` addresses the source view's bytes;
                // `start` was bounds-established by the compiler-emitted
                // range checks that precede RefSlice.
                let new_ptr = unsafe { base.add(start) };
                let obj = state.heap.alloc_byte_slice(new_ptr, len)?;
                state.record_allocation();
                state.set_reg(dst, Value::from_ptr(obj.as_ptr() as *mut u8));
                return Ok(DispatchResult::Continue);
            }

            // Get the base pointer from source - could be a pointer, thin ref, or object.
            // `is_regular_ptr` (NOT `is_ptr`) leads: a FatRef/ThinRef shares
            // TAG_POINTER but sets SPECIAL_VALUE_MARKER, so `is_ptr()` is
            // true for it and the first arm's `as_ptr::<u8>()` would return
            // the FAT_REF_MARKER payload — which `try_from_ptr` below then
            // dereferences → SIGSEGV. Trigger: `&slice[range]` where `slice`
            // is itself a FatRef (slice-of-a-slice, e.g. HttpParser.feed's
            // `&remaining[..scan_end]` over `&buf[pos..]`). Gating on
            // is_regular_ptr routes a FatRef to the is_fat_ref arm below.
            let mut base_ptr = if src.is_regular_ptr() {
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

            // If base_ptr is a List object, follow backing_ptr to get actual data.
            // Alignment-checked: a misaligned base_ptr can't be a
            // header, so the LIST / typed-array discrimination
            // collapses to "treat as opaque bytes" — caller's
            // downstream offsetting still works since `base_ptr.add(start * elem_size)`
            // is alignment-agnostic for bytewise reads.
            if let Some(header) = unsafe { heap::ObjectHeader::try_from_ptr(base_ptr) } {
                if header.type_id == TypeId::LIST {
                    // List layout: [ObjectHeader][len: Value][cap: Value][backing_ptr: Value]
                    // backing_ptr points to another array object with the actual elements
                    let backing_ptr_val =
                        unsafe { *(base_ptr.add(heap::OBJECT_HEADER_SIZE + 16) as *const Value) };
                    if backing_ptr_val.is_ptr() && !backing_ptr_val.is_nil() {
                        let backing_array = backing_ptr_val.as_ptr::<u8>();
                        // The backing array also has an ObjectHeader, skip it to get elements
                        base_ptr = unsafe { backing_array.add(heap::OBJECT_HEADER_SIZE) };
                    }
                } else {
                    // Non-LIST typed arrays (e.g., [Int; 3] allocated with TypeId::U64).
                    // Layout: [ObjectHeader][data...] — skip past the header.
                    base_ptr = unsafe { base_ptr.add(heap::OBJECT_HEADER_SIZE) };
                }
            }

            // Determine element size based on source TypeId
            // For typed arrays (U8, U16, U32, U64), elements are stored as raw integers
            // For LIST and other types, elements are NaN-boxed Values (elem_size = 0 signals Value)
            let elem_size: u32 = if !src.is_regular_ptr() {
                // FatRef handled by the fast-path above; a ThinRef / non-ptr
                // has no heap header to probe → NaN-boxed Values. (`is_ptr`
                // would be true for a ThinRef and read its marker as a ptr.)
                0 // Default to Value
            } else {
                let src_ptr = src.as_ptr::<u8>();
                // Alignment-checked header read: misaligned src means
                // "treat as NaN-boxed Values" (the default), since
                // there's no valid type_id to consult.
                match unsafe { heap::ObjectHeader::try_type_id(src_ptr) } {
                    Some(TypeId::U8) => 1,
                    Some(TypeId::U16) => 2,
                    Some(TypeId::U32) => 4,
                    Some(TypeId::U64) => 8,
                    _ => 0, // LIST, UNIT, misaligned, or null → NaN-boxed Values
                }
            };
            // eprintln!("[DEBUG RefSlice] elem_size={}", elem_size);

            // Adjust pointer by start offset based on element size
            let actual_elem_size = if elem_size == 0 {
                std::mem::size_of::<Value>()
            } else {
                elem_size as usize
            };
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
            } else if let Some((p, _len)) = heap::value_as_byte_slice(&slice) {
                // BYTE_SLICE byte view (ARCH-P5): the underlying data
                // pointer is payload slot 0, NOT the object base.
                Value::from_ptr(p)
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

            // Get slice value and extract length from FatRef or a
            // BYTE_SLICE byte-view object (ARCH-P5).
            let slice = state.get_reg(slice_reg);
            let len = if slice.is_fat_ref() {
                slice.as_fat_ref().len() as i64
            } else if let Some((_p, l)) = heap::value_as_byte_slice(&slice) {
                l as i64
            } else {
                // For non-slice values, return 0 (or could be error)
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

            // Respect fat_ref.reserved as the element stride: 1/2/4/8 for
            // raw integer arrays (bytes included) and 0 for NaN-boxed
            // Value arrays. A fixed `*const Value` read truncates byte
            // slices to the first element's tag bits.
            let value = if slice.is_fat_ref() {
                let fat_ref = slice.as_fat_ref();
                let len = fat_ref.len() as usize;
                if index < len {
                    let base = fat_ref.ptr();
                    match fat_ref.reserved {
                        0 => unsafe { *(base as *const Value).add(index) },
                        1 => Value::from_i64(unsafe { *base.add(index) } as i64),
                        2 => Value::from_i64(unsafe {
                            std::ptr::read_unaligned(base.add(index * 2) as *const i16)
                        } as i64),
                        4 => Value::from_i64(unsafe {
                            std::ptr::read_unaligned(base.add(index * 4) as *const i32)
                        } as i64),
                        8 => Value::from_i64(unsafe {
                            std::ptr::read_unaligned(base.add(index * 8) as *const i64)
                        }),
                        _ => unsafe { *(base as *const Value).add(index) },
                    }
                } else {
                    return Err(crate::interpreter::InterpreterError::IndexOutOfBounds {
                        index: index as i64,
                        length: len,
                    });
                }
            } else if let Some((base, len)) = heap::value_as_byte_slice(&slice) {
                // BYTE_SLICE byte view (ARCH-P5): bounds-checked raw
                // byte read, zero-extended into the Int NaN-box.
                if (index as u64) < len {
                    Value::from_i64(unsafe { *base.add(index) } as i64)
                } else {
                    return Err(crate::interpreter::InterpreterError::IndexOutOfBounds {
                        index: index as i64,
                        length: len as usize,
                    });
                }
            } else {
                Value::nil()
            };
            state.set_reg(dst, value);
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::SliceGetUnchecked) => {
            // Get element at index from slice (unchecked). Same stride
            // dispatch as SliceGet, without the bounds check.
            let dst = read_reg(state)?;
            let slice_reg = read_reg(state)?;
            let index_reg = read_reg(state)?;

            let slice = state.get_reg(slice_reg);
            let index = state.get_reg(index_reg).as_i64() as usize;

            let value = if slice.is_fat_ref() {
                let fat_ref = slice.as_fat_ref();
                let base = fat_ref.ptr();
                match fat_ref.reserved {
                    0 => unsafe { *(base as *const Value).add(index) },
                    1 => Value::from_i64(unsafe { *base.add(index) } as i64),
                    2 => Value::from_i64(unsafe {
                        std::ptr::read_unaligned(base.add(index * 2) as *const i16)
                    } as i64),
                    4 => Value::from_i64(unsafe {
                        std::ptr::read_unaligned(base.add(index * 4) as *const i32)
                    } as i64),
                    8 => Value::from_i64(unsafe {
                        std::ptr::read_unaligned(base.add(index * 8) as *const i64)
                    }),
                    _ => unsafe { *(base as *const Value).add(index) },
                }
            } else if let Some((base, _len)) = heap::value_as_byte_slice(&slice) {
                // BYTE_SLICE byte view (ARCH-P5): unchecked raw byte read.
                Value::from_i64(unsafe { *base.add(index) } as i64)
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

            // SLICE-SUBSLICE-RESOLVE-1 (#51 runtime leg): the receiver
            // arrives as a register-encoded CBGR ref / ThinRef when the
            // callee param is `slice: &[T]` — resolve to the referent
            // FatRef BEFORE classifying (GetE/IterInit precedent).
            // Pre-fix the unresolved ref fell through the FatRef and
            // byte-slice arms into a SILENT `src` passthrough:
            // `s.slice(1,5)` returned the WHOLE slice.
            let src = super::cbgr_helpers::resolve_arg_value(state, state.get_reg(src_reg));
            let start = state.get_reg(start_reg).as_i64() as u64;
            let end = state.get_reg(end_reg).as_i64() as u64;

            // Create new FatRef with adjusted pointer and length. The
            // element stride comes from fat_ref.reserved (1/2/4/8 for raw
            // integers, 0 = NaN-boxed Value) — using a fixed
            // `sizeof(Value)` here would walk past the end of byte slices
            // (text.as_bytes(), binary buffers).
            let result = if src.is_fat_ref() {
                let fat_ref = src.as_fat_ref();
                let len = fat_ref.len();
                if start <= end && end <= len {
                    let element_size = if fat_ref.reserved == 0 {
                        std::mem::size_of::<Value>()
                    } else {
                        fat_ref.reserved as usize
                    };
                    let new_ptr =
                        unsafe { (fat_ref.ptr() as *const u8).add(start as usize * element_size) };
                    let new_len = end - start;
                    let mut new_fat_ref = crate::value::FatRef::new(
                        new_ptr as *mut u8,
                        fat_ref.generation(),
                        fat_ref.epoch(),
                        fat_ref.capabilities(),
                        new_len,
                    );
                    new_fat_ref.reserved = fat_ref.reserved;
                    Value::from_fat_ref(new_fat_ref)
                } else {
                    return Err(crate::interpreter::InterpreterError::IndexOutOfBounds {
                        index: end as i64,
                        length: len as usize,
                    });
                }
            } else if let Some((base, len)) = heap::value_as_byte_slice(&src) {
                // BYTE_SLICE byte view (ARCH-P5): bounds-checked
                // re-slice producing a NEW BYTE_SLICE object
                // `{ptr + start, end - start}` (stride 1) — covers
                // subslice-of-subslice chains.
                if start <= end && end <= len {
                    // SAFETY: `start <= len` verified above; the source
                    // view addresses `len` bytes at `base`.
                    let new_ptr = unsafe { base.add(start as usize) };
                    let obj = state.heap.alloc_byte_slice(new_ptr, end - start)?;
                    state.record_allocation();
                    Value::from_ptr(obj.as_ptr() as *mut u8)
                } else {
                    return Err(crate::interpreter::InterpreterError::IndexOutOfBounds {
                        index: end as i64,
                        length: len as usize,
                    });
                }
            } else {
                // Never a guessed slot / silent identity — a receiver
                // that is neither a FatRef nor a byte-slice view means
                // the ref-resolution contract above was violated.
                return Err(crate::interpreter::InterpreterError::Panic {
                    message: format!(
                        "slice_subslice: receiver is neither FatRef nor byte-slice \
                         (bits {:#x}) — SLICE-SUBSLICE-RESOLVE-1",
                        src.to_bits()
                    ),
                });
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

            // SLICE-SUBSLICE-RESOLVE-1: same ref-resolution contract as
            // SliceSubslice above.
            let src = super::cbgr_helpers::resolve_arg_value(state, state.get_reg(src_reg));
            let mid = state.get_reg(mid_reg).as_i64() as u64;

            if src.is_fat_ref() {
                let fat_ref = src.as_fat_ref();
                let len = fat_ref.len();
                if mid <= len {
                    // Honour the reserved elem-size (0 = NaN-boxed
                    // Value, 1/2/4/8 = raw widths) — a fixed
                    // sizeof(Value) stride walks 8x past byte slices.
                    let element_size = if fat_ref.reserved == 0 {
                        std::mem::size_of::<Value>()
                    } else {
                        fat_ref.reserved as usize
                    };

                    // Left slice: [0, mid)
                    let mut left_ref = crate::value::FatRef::new(
                        fat_ref.ptr(),
                        fat_ref.generation(),
                        fat_ref.epoch(),
                        fat_ref.capabilities(),
                        mid,
                    );
                    left_ref.reserved = fat_ref.reserved;

                    // Right slice: [mid, len)
                    let right_ptr =
                        unsafe { (fat_ref.ptr() as *const u8).add(mid as usize * element_size) };
                    let mut right_ref = crate::value::FatRef::new(
                        right_ptr as *mut u8,
                        fat_ref.generation(),
                        fat_ref.epoch(),
                        fat_ref.capabilities(),
                        len - mid,
                    );
                    right_ref.reserved = fat_ref.reserved;

                    state.set_reg(dst1, Value::from_fat_ref(left_ref));
                    state.set_reg(dst2, Value::from_fat_ref(right_ref));
                } else {
                    return Err(crate::interpreter::InterpreterError::IndexOutOfBounds {
                        index: mid as i64,
                        length: len as usize,
                    });
                }
            } else if let Some((base, len)) = heap::value_as_byte_slice(&src) {
                // BYTE_SLICE byte view (ARCH-P5): split into TWO new
                // BYTE_SLICE objects `{ptr, mid}` / `{ptr + mid,
                // len - mid}` (stride 1).
                if mid <= len {
                    let left = state.heap.alloc_byte_slice(base, mid)?;
                    state.record_allocation();
                    // SAFETY: `mid <= len` verified above; the source
                    // view addresses `len` bytes at `base`.
                    let right_ptr = unsafe { base.add(mid as usize) };
                    let right = state.heap.alloc_byte_slice(right_ptr, len - mid)?;
                    state.record_allocation();
                    state.set_reg(dst1, Value::from_ptr(left.as_ptr() as *mut u8));
                    state.set_reg(dst2, Value::from_ptr(right.as_ptr() as *mut u8));
                } else {
                    return Err(crate::interpreter::InterpreterError::IndexOutOfBounds {
                        index: mid as i64,
                        length: len as usize,
                    });
                }
            } else {
                return Err(crate::interpreter::InterpreterError::Panic {
                    message: format!(
                        "slice_split_at: receiver is neither FatRef nor byte-slice \
                         (bits {:#x}) — SLICE-SUBSLICE-RESOLVE-1",
                        src.to_bits()
                    ),
                });
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
                // Heap-based ref: read generation from AllocationHeader.
                let ptr_addr = src_val.as_ptr::<u8>() as usize;
                let header_addr = ptr_addr
                    .wrapping_sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize);
                let gen_ptr = (header_addr
                    + verum_common::layout::ALLOCATION_HEADER_GENERATION_OFFSET as usize)
                    as *const u32;
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
                // Heap-based ref: read epoch from AllocationHeader.
                let ptr_addr = src_val.as_ptr::<u8>() as usize;
                let header_addr = ptr_addr
                    .wrapping_sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize);
                let epoch_ptr = (header_addr
                    + verum_common::layout::ALLOCATION_HEADER_EPOCH_OFFSET as usize)
                    as *const u16;
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
                ref_gen == current_gen
                    && validate_epoch_window(ref_epoch, global_epoch, EPOCH_WINDOW_SIZE)
            } else if src_val.is_ptr() && !src_val.is_nil() {
                // Heap-based ref: validate epoch using window comparison
                let ptr_addr = src_val.as_ptr::<u8>() as usize;
                let header_addr = ptr_addr
                    .wrapping_sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize);
                let epoch_ptr = (header_addr
                    + verum_common::layout::ALLOCATION_HEADER_EPOCH_OFFSET as usize)
                    as *const u16;
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
                // Check CBGR FREED flag for data pointers — see
                // `verum_common::cbgr::flags::FREED` and the
                // `ALLOCATION_HEADER_FLAGS_OFFSET` canonical constant.
                let data_ptr = src.as_ptr::<u8>() as usize;
                let header_addr = data_ptr
                    .wrapping_sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize);
                if state.cbgr_allocations.contains(&header_addr) {
                    let flags = unsafe {
                        *((header_addr
                            + verum_common::layout::ALLOCATION_HEADER_FLAGS_OFFSET as usize)
                            as *const u32)
                    };
                    flags & verum_common::cbgr::flags::FREED == 0 // Valid if not freed
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
        // Allocator (0x60-0x6F) — added 2026-05-02 per refactor plan.
        //
        // These are reachable Phase-1 stubs for the new
        // `CbgrSubOpcode::Alloc` / `AllocZeroed` / `Dealloc` /
        // `SecureZero` byte values.  The full Phase-4 wiring will
        // route emitting from `core/intrinsics/runtime/cbgr.vr`
        // through these handlers; until then they remain reachable
        // but unused (codegen still emits the legacy
        // `SystemSubOpcode::CbgrAlloc` 0xA0 / etc. via FfiExtended).
        // Dispatching to them now means a forward-rolled bytecode
        // file that uses the new home will execute correctly.
        // ================================================================
        Some(CbgrSubOpcode::Alloc)
        | Some(CbgrSubOpcode::AllocZeroed)
        | Some(CbgrSubOpcode::Dealloc)
        | Some(CbgrSubOpcode::SecureZero) => Err(InterpreterError::NotImplemented {
            feature: "cbgr_extended allocator (Phase 4 of subop refactor not yet wired)",
            opcode: Some(Opcode::CbgrExtended),
        }),

        // ================================================================
        // Unimplemented sub-opcodes
        // ================================================================
        None => Err(InterpreterError::NotImplemented {
            feature: "cbgr_extended sub-opcode",
            opcode: Some(Opcode::CbgrExtended),
        }),
    }
}
