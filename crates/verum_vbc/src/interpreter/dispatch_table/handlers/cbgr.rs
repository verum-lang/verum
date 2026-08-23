//! CBGR (Capability-Based Generational References) instruction handlers for VBC interpreter.

use super::bytecode_io::read_u8;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::heap;
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;
use super::envelope::dispatch_enveloped;
use super::cbgr_helpers::{
    CBGR_NO_CHECK_GENERATION, EPOCH_WINDOW_SIZE, decode_cbgr_ref, encode_cbgr_ref,
    encode_cbgr_ref_mut, is_cbgr_ref, is_cbgr_ref_mutable, regref_generation_matches,
    strip_cbgr_ref_mutability, validate_cbgr_generation, validate_epoch_window,
};
use crate::instruction::{CbgrSubOpcode, Opcode, Reg};
use super::ffi_extended::{ALLOC_ERR_INVALID_SIZE, ALLOC_ERR_OUT_OF_MEMORY, cbgr_header_generation_epoch, cbgr_legacy_alloc, cbgr_legacy_allocate, cbgr_user_allocate, cbgr_user_deallocate, cbgr_user_realloc, make_alloc_err, MAX_FFI_ALLOCATION_SIZE, value_as_addr};
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
    state.set_reg(dst, encode_cbgr_ref(abs_index, generation));
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
    if is_cbgr_ref(&src_val) && is_cbgr_ref_mutable(src_val) {
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
    state.set_reg(dst, encode_cbgr_ref_mut(abs_index, generation));
    Ok(DispatchResult::Continue)
}

/// Whole-value `Shared<T>` cell accessor (SHARED-STRONGCOUNT / T0374).
///
/// If `base_ptr` is a live `Shared<T>` carrier, return the address of its
/// inner value cell (slot1) — the SAME cell that `*shared` reads, so the
/// Deref read side and the DerefMut write side operate on ONE location and
/// can never drift. Returns `None` for any non-Shared pointer.
///
/// This is the WHOLE-VALUE twin of `memory_collections::shared_carrier_inner`,
/// and deliberately NOT the same helper: that one FOLLOWS slot1's pointer to
/// the inner OBJECT so a subsequent field offset (`shared.field`) lands inside
/// the inner `T`; this one hands back slot1's ADDRESS so a whole-value read
/// (`*shared`) / write (`*shared = X`) touches the cell itself. Reusing the
/// field-access helper here would target the inner object's header (or, for an
/// immediate inner `T`, the carrier header) — both wrong for a whole write.
///
/// Guard: a `Heap<T>` CBGR cell's data pointer addresses its inner Value
/// directly (there is no `ObjectHeader` there), so its out-of-bounds "header"
/// bits could coincide with `TypeId::SHARED`. The `cbgr_allocations`
/// membership test excludes those cells — the same live-allocation guard the
/// T0107 `handle_clone` SHARED arm uses.
///
/// Shared layout: `[ObjectHeader][refcount:i64 @ slot0][inner:Value @ slot1]`.
fn shared_inner_cell(state: &InterpreterState, base_ptr: *mut u8) -> Option<*mut Value> {
    if base_ptr.is_null() {
        return None;
    }
    // A CBGR `Heap<T>` cell is not a Shared carrier; its data pointer is not
    // an ObjectHeader, so never read a SHARED type_id out of it.
    let is_cbgr_cell = state.cbgr_allocations.contains(
        &(base_ptr as usize)
            .wrapping_sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize),
    );
    if is_cbgr_cell {
        return None;
    }
    if !(base_ptr as usize).is_multiple_of(std::mem::align_of::<heap::ObjectHeader>()) {
        return None;
    }
    // SAFETY: alignment verified; every VBC heap object begins with an
    // ObjectHeader.
    let header = unsafe { heap::ObjectHeader::ref_or_stub(base_ptr) };
    if header.type_id != TypeId::SHARED {
        return None;
    }
    // slot1 = the inner Value cell (skip the ObjectHeader and refcount slot0).
    // SAFETY: `Shared.new(...)` initializes slot1; the pointer stays within
    // the Shared object's data area.
    Some(unsafe { (base_ptr.add(heap::OBJECT_HEADER_SIZE) as *mut Value).add(1) })
}

/// Width of one packed scalar slot in a bridge allocation, in bytes.
///
/// `ptr_read<T>` / `ptr_write<T>` reach Tier 0 through ONE baked, fully
/// type-erased body (`core.intrinsics.memory.ptr_read` carries
/// `type_params = 0` in the archive and lowers to a single `Deref`), so `T`
/// is not recoverable at the instruction — 8 is the only width the opcode
/// can honestly name, and it is the width the AOT `Deref`/`DerefMut`
/// lowering uses for the same call.  Narrower typed access has its own
/// opcodes (`DerefRaw` / `DerefMutRaw` carry an explicit `size`).
const BRIDGE_SLOT_BYTES: usize = 8;

/// T0108 — resolve a raw address to a slot inside a live **bridge
/// allocation**, the packed byte block `cbgr_allocate` hands to Verum code.
///
/// One `*const T` fronts two physically different layouts: a packed bridge
/// buffer (raw bytes, shared verbatim with FFI and with AOT) and an array of
/// 8-byte NaN-boxed `Value` slots (List / slice backings).  They are not
/// distinguishable from the static type, so provenance is decided here, at
/// runtime, from the allocation index.
///
/// Returns `Ok(None)` when `addr` is not inside any live bridge payload —
/// the caller then keeps its existing behaviour for that value.  An address
/// that IS inside one but has fewer than [`BRIDGE_SLOT_BYTES`] bytes left is
/// an out-of-bounds access and reports rather than silently truncating.
fn bridge_scalar_slot(
    state: &InterpreterState,
    addr: usize,
    op: &'static str,
) -> InterpreterResult<Option<*mut u64>> {
    let Some((&user, &len)) = state.cbgr_bridge_extents.range(..=addr).next_back() else {
        return Ok(None);
    };
    let offset = addr - user;
    if offset >= len {
        // The nearest block below `addr` ends before it: `addr` belongs to
        // no live bridge allocation.
        return Ok(None);
    }
    if len - offset < BRIDGE_SLOT_BYTES {
        return Err(InterpreterError::InvalidOperand {
            message: format!(
                "{op}: {BRIDGE_SLOT_BYTES}-byte access at offset {offset} of a \
                 {len}-byte bridge allocation reads past its end",
            ),
        });
    }
    Ok(Some(addr as *mut u64))
}

/// Remaining capacity of the live bridge allocation that contains `addr`.
///
/// Sibling of [`bridge_scalar_slot`] for WHOLE-RECORD access: the scalar
/// form demands 8 readable bytes, a record needs as many as its object
/// header says it occupies, so the caller must see the room rather than
/// a fixed-width slot.
///
/// `None` when `addr` lies in no live bridge payload — the same
/// "provenance from the allocation index" answer, and the ONE fact that
/// distinguishes the two physical layouts a `&unsafe T` can front
/// (T0108, T0705).
pub(super) fn bridge_extent_room(
    state: &InterpreterState,
    addr: usize,
) -> Option<usize> {
    let (&user, &len) = state.cbgr_bridge_extents.range(..=addr).next_back()?;
    let offset = addr.checked_sub(user)?;
    if offset >= len {
        return None;
    }
    Some(len - offset)
}

/// The raw 8-byte pattern a scalar `Value` occupies in packed memory.
///
/// Bridge allocations are byte-addressable storage shared with `memcpy` /
/// `memset` / `load_byte` and with AOT-compiled code, so a scalar is stored
/// FLAT — `42` occupies the bytes of `42`, not the bytes of its NaN box.
/// That is what makes a Tier-0 `ptr_write` byte-observable through the
/// `mem_raw` intrinsics and byte-identical to the AOT store.
///
/// `None` for a non-scalar (heap object, `Text`, reference, boxed carrier):
/// its NaN box is a tagged pointer into interpreter-private storage, and the
/// flat/boxed ambiguity is not decidable on the way back out — `i64::MAX`
/// stored flat has the same bit pattern as a NaN-boxed header.  The caller
/// reports instead of storing something it could not read back.
///
/// `Int128` is `None` for the same reason in the other direction: 128 bits
/// do not fit a [`BRIDGE_SLOT_BYTES`] slot, and narrowing to the low 64
/// would be a silent half-store.
fn packed_scalar_bits(value: Value) -> Option<u64> {
    if value.is_boxed_i128() {
        None
    } else if value.is_int() {
        Some(value.as_i64() as u64)
    } else if value.is_float() {
        Some(value.as_f64().to_bits())
    } else if value.is_bool() {
        Some(value.as_bool() as u64)
    } else if value.is_unit() || value.is_nil() {
        Some(0)
    } else {
        None
    }
}

/// T0108 — the bridge slot an Int-tagged reference value addresses, if any.
///
/// Shared by the `Deref` and `DerefMut` arms so read and write agree
/// exactly on which values count as bridge addresses.  Only an Int-tagged
/// value is considered: CBGR register-refs are Int-tagged too but are
/// recognised earlier by `is_cbgr_ref`, and every other tag (pointer,
/// ThinRef, FatRef, nil) has its own arm above.  A non-positive payload is
/// not an address, so it never reaches the index.
fn int_tagged_bridge_slot(
    state: &InterpreterState,
    ref_val: Value,
    op: &'static str,
) -> InterpreterResult<Option<*mut u64>> {
    if !ref_val.is_int() {
        return Ok(None);
    }
    let addr = ref_val.as_i64();
    if addr <= 0 {
        return Ok(None);
    }
    bridge_scalar_slot(state, addr as usize, op)
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
            } else if bridge_extent_room(state, ptr_addr).is_some() {
                // BRIDGE MEMORY IS DATA (T0705/T0384). An address inside
                // a live bridge block names a `Value` slot, so `*p` READS
                // it. The identity arm below is for a CBGR base pointer —
                // an allocation header, where `*p` means "the struct
                // itself" — and answering that here handed `*s.get()` the
                // raw address, which the display then rendered by reading
                // it as an object header: `<object type_id=42>`, where 42
                // is the stored value itself.
                // SAFETY: the extent proves a readable `Value` at `ptr`.
                let value = unsafe { *(ptr_addr as *const Value) };
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
                // ONE whole-value Shared authority with the DerefMut write
                // side (T0374): both read/write slot1 via `shared_inner_cell`.
                if let Some(slot1) = shared_inner_cell(state, base_ptr) {
                    // SAFETY: `slot1` addresses the initialized inner Value
                    // cell of a live Shared carrier (guaranteed by the helper).
                    let inner = unsafe { *slot1 };
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
        let (abs_index, generation) = decode_cbgr_ref(ref_val);
        // CBGR generation validation (Tier 0 only; skipped for Tier 1/2 sentinel)
        validate_cbgr_generation(state, abs_index, generation)?;
        let value = state.registers.get_absolute(abs_index);
        state.set_reg(dst, value);
    } else if let Some(slot) = int_tagged_bridge_slot(state, ref_val, "ptr_read")? {
        // **T0108 — Int-tagged BRIDGE address.**  `cbgr_allocate` returns
        // its user pointer Int-tagged (`Value::from_i64`), so a
        // `ptr_read(p)` over bridge memory arrives here, not in the
        // pointer-tagged arm above.  Without this the address fell through
        // to the identity fallback and `ptr_read` handed back the POINTER
        // instead of the pointee — an identity stub that reported success.
        //
        // Packed storage, so the read is the flat 8-byte scalar the write
        // side laid down (and the same bytes the AOT `Deref` loads), never
        // a NaN box.
        //
        // SAFETY: `bridge_scalar_slot` proved the full 8 bytes lie inside a
        // live bridge payload (the extent leaves the index on free).
        let raw = unsafe { std::ptr::read_unaligned(slot) };
        state.set_reg(dst, Value::from_i64(raw as i64));
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
/// Store `value` FLAT into a live bridge allocation at `addr`.
///
/// ONE authority for both arms of `handle_deref_mut` — the Int-tagged
/// bridge address `cbgr_allocate` hands back, and the Ptr-tagged one the
/// same address becomes after `ptr as &unsafe T`. Bridge storage is
/// byte-addressable memory shared verbatim with `memcpy` / `load_byte`
/// and with AOT code, so a scalar occupies its own bytes and a record
/// occupies its field slots — never a NaN box, which is a tag into
/// interpreter-private storage.
///
/// The copy is bounded on BOTH ends: by the object header's own
/// data-section `size` and by the room left in the extent.
pub(super) fn bridge_flat_store(
    state: &mut InterpreterState,
    addr: *mut u8,
    room: usize,
    value: Value,
    op: &'static str,
) -> InterpreterResult<DispatchResult> {
    if let Some(bits) = packed_scalar_bits(value) {
        if room < BRIDGE_SLOT_BYTES {
            return Err(InterpreterError::InvalidOperand {
                message: format!(
                    "{op}: {BRIDGE_SLOT_BYTES}-byte store into a bridge allocation                      with only {room} byte(s) left writes past its end",
                ),
            });
        }
        // SAFETY: the extent proved 8 readable/writable bytes at `addr`.
        unsafe {
            std::ptr::write_unaligned(addr as *mut u64, bits);
        }
    } else if value.is_ptr()
        && !value.is_nil()
        && state
            .heap
            .contains(value.as_ptr::<u8>() as *const heap::ObjectHeader)
    {
        // OWNERSHIP BEFORE INSPECTION.  `try_from_ptr` validates the SHAPE of
        // an address (marker bit clear, 8-aligned) and then DEREFERENCES it to
        // answer — so for a well-formed address that names nothing it does not
        // return `None`, it reads unmapped memory.  `Heap::contains` is the
        // ownership authority, and its own doc states the rule this arm was
        // breaking: "code that needs to inspect headers safely … must consult
        // this method first".  The sibling inspection site (`handle_clone`,
        // memory_collections.rs) already does.
        //
        // Without it the `non_scalar_bridge_store_reports_instead_of_corrupting`
        // regression SIGSEGV'd reading `header.size` at 0x100c — the guard that
        // exists to REPORT this exact input could not survive evaluating it, so
        // a reportable store crashed the interpreter instead.  A pointer the
        // heap does not own (a system buffer from `MemExtended::Alloc`, which
        // carries no header at all, or a corrupted register) now falls to the
        // reporting arm below rather than being read through.
        let obj = value.as_ptr::<u8>();
        let Some(header) = (unsafe { heap::ObjectHeader::try_from_ptr(obj) }) else {
            return Err(InterpreterError::InvalidOperand {
                message: format!(
                    "{op}: cannot store a non-scalar value ({:?}) into a packed                      bridge allocation — its NaN box is a tag into                      interpreter-private storage and is not recoverable by the                      matching read",
                    value.tag()
                ),
            });
        };
        let bytes = (header.size as usize).min(room);
        if bytes > 0 {
            // SAFETY: `room` bounds the destination inside a live bridge
            // payload and `header.size` bounds the source inside the
            // object's data section; the regions are distinct allocations.
            unsafe {
                std::ptr::copy_nonoverlapping(obj.add(heap::OBJECT_HEADER_SIZE), addr, bytes);
            }
        }
    } else {
        return Err(InterpreterError::InvalidOperand {
            message: format!(
                "{op}: cannot store a non-scalar value ({:?}) into a packed bridge                  allocation",
                value.tag()
            ),
        });
    }
    state.cbgr_epoch = state.cbgr_epoch.wrapping_add(1);
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_deref_mut(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let ref_reg = read_reg(state)?;
    let value_reg = read_reg(state)?;
    let ref_val = state.get_reg(ref_reg);
    let value = state.get_reg(value_reg);
    if std::env::var("VERUM_TRACE_PTRWRITE").is_ok() {
        let addr = if ref_val.is_ptr() {
            ref_val.as_ptr::<u8>() as usize
        } else if ref_val.is_int() {
            ref_val.as_i64() as usize
        } else {
            0
        };
        eprintln!(
            "[ptrwrite] tag={:?} thin={} fat={} cbgrref={} ptr={} int={} addr={:#x} room={:?} \
             val_tag={:?}",
            ref_val.tag(),
            ref_val.is_thin_ref(),
            ref_val.is_fat_ref(),
            is_cbgr_ref(&ref_val),
            ref_val.is_ptr(),
            ref_val.is_int(),
            addr,
            bridge_extent_room(state, addr),
            value.tag()
        );
    }

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
        let (abs_index, generation) = decode_cbgr_ref(ref_val);
        validate_cbgr_generation(state, abs_index, generation)?;
        state.registers.set_absolute(abs_index, value);
        // CBGR epoch advancement: mutation through reference advances the epoch
        // This enables temporal ordering detection for stale references
        state.cbgr_epoch = state.cbgr_epoch.wrapping_add(1);
    } else if ref_val.is_ptr() && !ref_val.is_nil() {
        // Heap pointer deref-mut: write value at pointer location.
        let base_ptr = ref_val.as_ptr::<u8>();
        // **T0705 — the TAG does not name the LAYOUT.**
        // A bridge address may arrive Int-tagged (as `cbgr_allocate`
        // returns it) or Ptr-tagged (after `ptr as &unsafe T`, which is
        // how every stdlib builder spells it). The destination's
        // PROVENANCE decides how it must be written, not the tag on the
        // register — so the same flat store the Int-tagged arm performs
        // has to happen here too, or the identical program writes a
        // boxed Value or raw bytes depending on a cast.
        if let Some(room) = bridge_extent_room(state, base_ptr as usize) {
            return bridge_flat_store(state, base_ptr, room, value, "ptr_write");
        }
        // **T0374 — DerefMut twin of the Deref SHARED arm.**  `*shared = X`
        // must write THROUGH the carrier to the inner value cell (slot1); a
        // naive write at `base_ptr` lands on the Shared ObjectHeader and is
        // lost.  `shared_inner_cell` redirects only genuine Shared carriers —
        // non-Shared pointers (incl. `Heap<T>` CBGR cells, whose data pointer
        // already IS the inner cell) keep the identity write.  ONE whole-value
        // Shared authority shared with the Deref read side above.
        let write_ptr =
            shared_inner_cell(state, base_ptr).unwrap_or(base_ptr as *mut Value);
        // SAFETY: `write_ptr` is either `base_ptr` (a live heap Value slot) or
        // the guarded inner cell of a Shared carrier.
        unsafe {
            std::ptr::write(write_ptr, value);
        }
        // CBGR epoch advancement on heap mutation
        state.cbgr_epoch = state.cbgr_epoch.wrapping_add(1);
        // Update the epoch in the AllocationHeader for this allocation.
        // Keyed off the ORIGINAL carrier pointer — a Shared carrier is not a
        // CBGR allocation, so this stays a no-op for the redirected case.
        // Header sits immediately before the data payload — see
        // `verum_common::layout::ALLOCATION_HEADER_SIZE` and
        // `ALLOCATION_HEADER_EPOCH_OFFSET`.
        let header_addr = (base_ptr as usize)
            .wrapping_sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize);
        if state.cbgr_allocations.contains(&header_addr) {
            unsafe {
                let epoch_ptr = (header_addr
                    + verum_common::layout::ALLOCATION_HEADER_EPOCH_OFFSET as usize)
                    as *mut u16;
                *epoch_ptr = state.cbgr_epoch as u16;
            }
        }
    } else if let Some(slot) = int_tagged_bridge_slot(state, ref_val, "ptr_write")? {
        // **T0108 — Int-tagged BRIDGE address (the silent-no-op leg).**
        // `cbgr_allocate` returns its user pointer Int-tagged, so every
        // `ptr_write` over bridge memory landed here — where, before this
        // arm existed, the `if/else if` chain simply ENDED.  The handler
        // returned `Continue` having written nothing: the store vanished
        // and the program reported success.
        //
        // Packed storage: the scalar goes in FLAT, so `load_byte` /
        // `memcpy` observe the value's own bytes and the AOT store of the
        // same program produces the same memory.
        // The Int-tagged bridge address `cbgr_allocate` returns —
        // same store, ONE authority (see `bridge_flat_store`).
        if let Some(room) = bridge_extent_room(state, ref_val.as_i64() as usize) {
            return bridge_flat_store(state, slot as *mut u8, room, value, "ptr_write");
        }
        let Some(bits) = packed_scalar_bits(value) else {
            return Err(InterpreterError::InvalidOperand {
                message: format!(
                    "ptr_write: cannot store a non-scalar value ({:?}) into a packed \
                     bridge allocation — its NaN box is a tag into interpreter-private \
                     storage and is not recoverable by the matching ptr_read",
                    value.tag()
                ),
            });
        };
        // SAFETY: `bridge_scalar_slot` proved the full 8 bytes lie inside a
        // live bridge payload (the extent leaves the index on free).
        unsafe {
            std::ptr::write_unaligned(slot, bits);
        }
        // Epoch advancement + header epoch stamp, exactly as the
        // pointer-tagged arm above does for the same allocations — the
        // tag a bridge address happens to carry must not change whether a
        // mutation is temporally observable.
        state.cbgr_epoch = state.cbgr_epoch.wrapping_add(1);
        let header_addr = (ref_val.as_i64() as usize)
            .wrapping_sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize);
        if state.cbgr_allocations.contains(&header_addr) {
            // SAFETY: membership proves a live 32-byte AllocationHeader at
            // `header_addr`; the offset is the canonical layout constant.
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
        let (abs_index, generation) = decode_cbgr_ref(ref_val);
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
        let (abs_index, generation) = decode_cbgr_ref(ref_val);
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
        encode_cbgr_ref(abs_index, CBGR_NO_CHECK_GENERATION),
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
        encode_cbgr_ref(abs_index, CBGR_NO_CHECK_GENERATION),
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

            // **T0202 TENSOR-HANDLE-OBJECT-1 (drop leg)** — the tensor
            // carrier reclaims its payload when the owning binding
            // drops: the inline `TensorHandle` is replaced by the
            // empty handle and dropped (its `Drop` decrefs, freeing
            // the shared `TensorData` when this was the last owner).
            // `take_and_drop_payload` is idempotent, so alias
            // bindings' later DropRefs are no-ops on a dead-but-valid
            // handle.  The carrier OBJECT stays alive for the
            // interpreter heap to reclaim (`Heap::clear` runs the same
            // glue at teardown), mirroring Shared's no-hard-free
            // policy; control falls through to the builtin CBGR
            // cleanup below (register-generation bump + clear).
            if type_id == crate::types::TypeId::TENSOR {
                let payload = unsafe {
                    obj_ptr.add(heap::OBJECT_HEADER_SIZE)
                        as *mut crate::interpreter::tensor::TensorHandle
                };
                // SAFETY: the TENSOR header proves an initialized
                // inline `TensorHandle` payload (the
                // `alloc_tensor_value` contract).
                unsafe { crate::interpreter::tensor::take_and_drop_payload(payload) };
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

            // DROP-GLUE-TYPEID-1 (runtime leg): layout-plausibility gate.
            // `is_regular_ptr()` cannot distinguish a heap-object BASE
            // pointer from an INTERIOR pointer (`&obj.field as *const T`
            // locals in baked stdlib bytecode — `AtomicU64.load`'s `ptr`)
            // or a stale pointer over reused memory.  For those, the
            // "header" read above is pointee DATA: a garbage `type_id`
            // word that can coincide with a real drop-carrying descriptor
            // (observed live: `RwLockWriteGuard.drop` executing against
            // Float bits → "method 'RwLock.release_write' not found on
            // receiver of runtime kind `Float`"; `Weak.drop` →
            // NullPointerAt; `WindowsCondvar.drop` → field-OOB on a fake
            // header with size=1).  The resolved-name guard below cannot
            // reject these — the foreign drop IS a genuine `.drop`.  The
            // discriminating fact is the header's `size` word: a REAL
            // allocation for the descriptor has an exact, enumerable size
            // (`ObjectHeader::layout_matches_descriptor`); garbage almost
            // never does.  Implausible → treat as no-descriptor and fall
            // through to the builtin CBGR cleanup, which operates on raw
            // bits.  The codegen twin fix stops emitting DropRef for
            // raw-pointer bindings at the source; this gate protects
            // bytecode baked BEFORE that fix and every other stale-pointer
            // route into DropRef.
            let type_desc_idx = type_desc_idx.filter(|idx| {
                let plausible = state
                    .module
                    .types
                    .get(*idx)
                    .zip(unsafe { heap::ObjectHeader::try_from_ptr(header_ptr) })
                    .map(|(desc, header)| header.layout_matches_descriptor(desc))
                    .unwrap_or(false);
                if !plausible && std::env::var("VERUM_TRACE_DROPFN").is_ok() {
                    let tn = state
                        .module
                        .types
                        .get(*idx)
                        .and_then(|td| state.module.strings.get(td.name))
                        .unwrap_or("?");
                    let hdr_size = unsafe { heap::ObjectHeader::try_from_ptr(header_ptr) }
                        .map(|h| h.size)
                        .unwrap_or(u32::MAX);
                    eprintln!(
                        "[DROPFN] LAYOUT-REJECT type='{}' (id={}) header_size={} — implausible drop target, builtin cleanup",
                        tn, type_id.0, hdr_size
                    );
                }
                plausible
            });

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
                *(base_ptr.add(heap::LIST_PTR_OFFSET) as *const Value)
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
    dispatch_enveloped(state, cbgr_extended_body)
}

/// `CbgrExtended` sub-op arms. Invoked through
/// [`dispatch_enveloped`](super::envelope::dispatch_enveloped), which owns the
/// sub-op byte, the operand-length envelope and the pc reposition — an arm may
/// read any number of operands, and may `return` early, without desynchronising
/// the instruction stream.
/// Validate a ThinRef/FatRef STRUCTURE by its address (T0846) — the
/// Tier-0 twin of the AOT's `verum_cbgr_check` / `verum_cbgr_check_write`
/// / `verum_cbgr_check_fat` (a FatRef begins with its ThinRef, so the
/// thin fields sit at the same offsets).
///
/// Layout at `ref_addr`: `{user_ptr@0: u64, generation@8: u32,
/// epoch_caps@12: u32}` with `epoch_caps = epoch(low16) | caps(high16)`.
/// The verdict is: the referenced allocation is live (tracked) AND its
/// header's gen@8/epoch@12 match the ThinRef's — plus, when
/// `require_write`, the WRITE bit (0x02) in the caps half.
///
/// Reading the structure itself is the caller's unsafe contract (the
/// .vr intrinsics are `unsafe fn`); the HEADER read is gated on
/// `cbgr_allocations` membership like every other header access here.
fn thin_ref_struct_check(state: &InterpreterState, ref_addr: i64, require_write: bool) -> bool {
    use verum_common::layout as l;
    if ref_addr <= 0 {
        return false;
    }
    // SAFETY: dereferencing the caller-supplied structure address is the
    // documented contract of the `unsafe fn cbgr_check*` intrinsics.
    let (user_ptr, ref_gen, epoch_caps) = unsafe {
        let base = ref_addr as usize;
        (
            *(base as *const u64),
            *((base + l::THIN_REF_GENERATION_OFFSET as usize) as *const u32),
            *((base + l::THIN_REF_EPOCH_CAPS_OFFSET as usize) as *const u32),
        )
    };
    let hdr = l::ALLOCATION_HEADER_SIZE as usize;
    if user_ptr == 0 || (user_ptr as usize) < hdr {
        return false;
    }
    let header_addr = user_ptr as usize - hdr;
    if !state.cbgr_allocations.contains(&header_addr) {
        return false;
    }
    // SAFETY: liveness gated by the membership check above.
    let (actual_gen, actual_epoch) = unsafe {
        (
            *((header_addr + l::ALLOCATION_HEADER_GENERATION_OFFSET as usize) as *const u32),
            *((header_addr + l::ALLOCATION_HEADER_EPOCH_OFFSET as usize) as *const u16),
        )
    };
    let ref_epoch = (epoch_caps & 0xFFFF) as u16;
    let caps = (epoch_caps >> 16) as u16;
    actual_gen == ref_gen
        && actual_epoch == ref_epoch
        && (!require_write || caps & 0x02 != 0)
}

fn cbgr_extended_body(
    state: &mut InterpreterState,
    sub_op_byte: u8,
) -> InterpreterResult<DispatchResult> {
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
                let (abs_index, _gen) = decode_cbgr_ref(list_val);
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

            // Receiver resolution through the ONE layered authority
            // (T0705): the private one-hop chain here returned NIL for
            // a receiver forwarded through two method frames, and every
            // baked `Shared` &mut-self chain died on the reference-typed
            // field it then failed to read. Fat-refs keep their local
            // arm — resolve_receiver does not flatten them.
            let base_val = if base_val.is_fat_ref() {
                Value::from_ptr(base_val.as_fat_ref().ptr())
            } else {
                super::cbgr_helpers::resolve_receiver(state, base_val)
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

        Some(CbgrSubOpcode::RefFieldNamed) => {
            // #42: interior reference to a field resolved BY NAME at
            // runtime — the `&obj.field` twin of GetFieldNamed /
            // SetFieldNamed for receivers the codegen could not type
            // (a positional RefField on a GUESSED index is an interior
            // pointer into the WRONG field). Same resolver authorities
            // as the by-name read/write doors (resolve_arg_value +
            // field_named_object + field_named_index) so the three
            // by-name doors can never drift; same interior product as
            // RefField (bounds check + cbgr_mutable_ptrs registration).
            // Format: dst:reg, base:reg, name:varint(StringId).
            let dst = read_reg(state)?;
            let base_reg = read_reg(state)?;
            let name_sid = read_varint(state)? as u32;

            let base_val =
                super::cbgr_helpers::resolve_arg_value(state, state.get_reg(base_reg));
            let (tid, base_ptr) =
                super::extended::field_named_object(state, base_val, "RefFieldNamed")?;
            let field_idx =
                super::extended::field_named_index(&state.module, tid, name_sid, "RefFieldNamed")?;

            // SAFETY: field_named_object verified a live header; the
            // descriptor-resolved index is in-bounds for the declared
            // layout, and the size gate below re-checks the concrete
            // allocation.
            let header = unsafe { heap::ObjectHeader::ref_or_stub(base_ptr) };
            let field_offset = field_idx * std::mem::size_of::<Value>();
            if field_offset + std::mem::size_of::<Value>() > header.size as usize {
                return Err(InterpreterError::Panic {
                    message: format!(
                        "RefFieldNamed: field {} (offset {}) exceeds object data size {}",
                        field_idx, field_offset, header.size
                    ),
                });
            }
            let field_ptr =
                unsafe { base_ptr.add(heap::OBJECT_HEADER_SIZE + field_offset) };
            state.cbgr_mutable_ptrs.insert(field_ptr as usize);
            state.set_reg(dst, Value::from_ptr(field_ptr as *mut u8));
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

            // Format: dst:reg, ptr:reg, len:reg[, elem:byte]
            // #48 phase-2: the optional 4th operand is the ELEMENT
            // STRIDE the emitter derived from the ptr arg's spelling
            // (`&unsafe Byte` → 1; `*const T`/Value backings → 8).
            // Legacy 3-operand streams keep the historic byte
            // semantics.
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let len_reg = read_reg(state)?;
            // Fixed 4-operand format (emitter always writes the stride
            // byte; the operand stream is self-describing so an optional
            // read would desynchronise on legacy 3-operand streams —
            // instead the PRECOMPILE_SCHEMA_VERSION bump forces a rebake).
            let elem_raw = read_u8(state)?;
            let elem: u8 = if matches!(elem_raw, 1 | 2 | 4 | 8) {
                elem_raw
            } else {
                1
            };

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
            // reserved carries the stride (0 = NaN-boxed Value slots —
            // the 8-byte case IS the Value-array case for raw parts).
            fat_ref.reserved = if elem == 8 { 0 } else { elem as u32 };

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
                        unsafe { *(base_ptr.add(heap::LIST_PTR_OFFSET) as *const Value) };
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
                    let attenuated = strip_cbgr_ref_mutability(src_val);
                    state.set_reg(dst, attenuated);
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
                let (abs_index, _) = decode_cbgr_ref(src);
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
                let is_mut = is_cbgr_ref_mutable(src_val);
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
                let is_mut = is_cbgr_ref_mutable(src_val);
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
                let shared = strip_cbgr_ref_mutability(src);
                state.set_reg(dst, shared);
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
                let (abs_index, generation) = decode_cbgr_ref(src);
                let exclusive = encode_cbgr_ref_mut(abs_index, generation);
                state.set_reg(dst, exclusive);
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
                let (_, ref_gen) = decode_cbgr_ref(src_val);
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
                let (abs_index, _) = decode_cbgr_ref(src_val);
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
                let (abs_index, ref_gen) = decode_cbgr_ref(src_val);
                let current_gen = state.registers.get_generation(abs_index);
                let ref_epoch = state.registers.get_epoch(abs_index);
                let global_epoch = state.registers.global_epoch();
                // T0367: register-refs carry a 22-bit generation — compare via
                // the SOLE modulo-2^22 authority (matches the deref validator).
                regref_generation_matches(ref_gen, current_gen)
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
            //
            // TWO models, disambiguated by the VALUE (T0846):
            //  * Int — `cbgr_check(thin_ref_ptr)`: the ADDRESS of a
            //    ThinRef structure; validate its generation+epoch
            //    against the allocation header, exactly like the AOT's
            //    verum_cbgr_check.  Yields Int 1/0 (the .vr signature
            //    returns Int).
            //  * pointer / register-ref — the legacy value model
            //    (FREED flag / register generation), yielding Bool.
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src = state.get_reg(src_reg);
            if src.is_int() {
                let verdict = thin_ref_struct_check(state, src.as_i64(), false);
                state.set_reg(dst, Value::from_i64(verdict as i64));
                return Ok(DispatchResult::Continue);
            }
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
                let (abs_index, generation) = decode_cbgr_ref(src);
                if generation == CBGR_NO_CHECK_GENERATION {
                    true
                } else {
                    let current_gen = state.registers.get_generation(abs_index);
                    // T0367: register-refs carry a 22-bit generation — compare
                    // via the SOLE modulo-2^22 authority.
                    regref_generation_matches(generation, current_gen)
                }
            } else {
                false
            };
            state.set_reg(dst, Value::from_bool(is_valid));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::RefCount) => {
            // Read the canonical rc@24 refcount from the allocation
            // header of a user pointer (T0846: rc@24 is canonical on
            // BOTH tiers — allocation stamps 1, RefRelease decrements).
            // The pre-fix arm ignored src and returned a constant 1,
            // which diverged from the AOT's header read.
            // Register-model refs have no header — they answer 1
            // (single owner), and so does an untracked address (the
            // AOT twin reads whatever is at ptr-32; the interpreter
            // will not dereference memory it does not own).
            // Format: dst:reg, src:reg
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let user = state.get_reg(src_reg).as_integer_compatible();
            let hdr = verum_common::layout::ALLOCATION_HEADER_SIZE as usize;
            let rc = if user > 0
                && (user as usize) >= hdr
                && state.cbgr_allocations.contains(&((user as usize) - hdr))
            {
                // SAFETY: membership in cbgr_allocations gates liveness;
                // rc@24 was written by cbgr_user_allocate.
                unsafe {
                    *(((user as usize) - hdr
                        + verum_common::layout::ALLOCATION_HEADER_REF_COUNT_OFFSET as usize)
                        as *const u32) as i64
                }
            } else {
                1
            };
            state.set_reg(dst, Value::from_i64(rc));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::CheckFat) | Some(CbgrSubOpcode::CheckWrite) => {
            // cbgr_check_fat(fat_ref_ptr) / cbgr_check_write(thin_ref_ptr).
            // The operand is the ADDRESS of a ThinRef/FatRef STRUCTURE
            // (a FatRef begins with its ThinRef, so the thin fields sit
            // at the same offsets): {user_ptr@0, generation@8,
            // epoch_caps@12 = epoch(low16) | caps(high16)}.  Mirrors
            // the AOT's verum_cbgr_check_fat / verum_cbgr_check_write:
            // validate generation+epoch against the allocation header;
            // CheckWrite additionally requires the WRITE bit (0x02) in
            // the caps half.  Format: dst:reg, ref_ptr:reg
            let is_write = matches!(sub_op, Some(CbgrSubOpcode::CheckWrite));
            let dst = read_reg(state)?;
            let ref_reg = read_reg(state)?;
            let ref_addr = state.get_reg(ref_reg).as_integer_compatible();
            let verdict = thin_ref_struct_check(state, ref_addr, is_write);
            state.set_reg(dst, Value::from_i64(verdict as i64));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // CBGR Management (0x50-0x5F)
        // ================================================================
        Some(CbgrSubOpcode::NewGeneration) => {
            // Advance the global generation counter and return the NEW
            // id — the Tier-0 twin of the AOT's
            // verum_ir_generation_counter (both start at 1; both hand
            // out 2, 3, … in call order).  The pre-fix arm returned
            // `epoch + 1`, which repeated the same id until an epoch
            // advance and could collide with epoch numbering (T0846).
            // Format: dst:reg
            let dst = read_reg(state)?;
            state.cbgr_generation_counter = state.cbgr_generation_counter.wrapping_add(1);
            state.set_reg(dst, Value::from_i64(state.cbgr_generation_counter as i64));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::Invalidate) => {
            // Format: src:reg
            // TWO models, disambiguated by the VALUE (T0846):
            //  * user pointer into a live bridge allocation → bump the
            //    canonical gen@8 header slot, revoking every
            //    outstanding heap reference (the cbgr.vr semantics of
            //    `cbgr_invalidate(user_ptr)`; the AOT twin is
            //    verum_cbgr_invalidate);
            //  * anything else → the legacy register-slot generation
            //    bump (references captured with the old generation
            //    fail validation on dereference).
            let src_reg = read_reg(state)?;
            let user = state.get_reg(src_reg).as_integer_compatible();
            let hdr = verum_common::layout::ALLOCATION_HEADER_SIZE as usize;
            if user > 0
                && (user as usize) >= hdr
                && state.cbgr_allocations.contains(&((user as usize) - hdr))
            {
                // SAFETY: membership in cbgr_allocations gates liveness;
                // gen@8 was written by cbgr_user_allocate.
                unsafe {
                    let gen_ptr = ((user as usize) - hdr
                        + verum_common::layout::ALLOCATION_HEADER_GENERATION_OFFSET as usize)
                        as *mut u32;
                    *gen_ptr = (*gen_ptr).wrapping_add(1);
                }
            } else {
                let abs_index = state.reg_base() + src_reg.0 as u32;
                state.registers.bump_generation(abs_index);
            }
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::EpochBegin) => {
            // Advance the global epoch and return the NEW value — the
            // read-modify-write twin of AdvanceEpoch (returns nothing)
            // + CurrentEpoch (reads only).  Carrier of
            // `cbgr_epoch_begin()`; the AOT twin advances the
            // global_epoch global.  Format: dst:reg
            let dst = read_reg(state)?;
            state.cbgr_epoch = state.cbgr_epoch.wrapping_add(1);
            state.set_reg(dst, Value::from_i64(state.cbgr_epoch as i64));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::Revoke) => {
            // cbgr_revoke(user_ptr): invalidate + deallocate in one
            // step (cbgr.vr).  Generation bump FIRST — the dealloc
            // sets FREED and releases the block.  Untracked pointers
            // are a no-op on both halves.  Format: ptr:reg (void)
            let src_reg = read_reg(state)?;
            let user = state.get_reg(src_reg).as_integer_compatible();
            let hdr = verum_common::layout::ALLOCATION_HEADER_SIZE as usize;
            if user > 0
                && (user as usize) >= hdr
                && state.cbgr_allocations.contains(&((user as usize) - hdr))
            {
                // SAFETY: membership gates liveness; gen@8 was written
                // by cbgr_user_allocate.
                unsafe {
                    let gen_ptr = ((user as usize) - hdr
                        + verum_common::layout::ALLOCATION_HEADER_GENERATION_OFFSET as usize)
                        as *mut u32;
                    *gen_ptr = (*gen_ptr).wrapping_add(1);
                }
                super::ffi_extended::cbgr_user_deallocate(state, user);
            }
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::RegisterRoot) => {
            // cbgr_register_root(user_ptr): accounting no-op — no
            // collector consumes roots on either tier (the AOT twin
            // verum_cbgr_register_root is an empty body).  The DEFINED
            // observable behaviour is: operand evaluated, no crash,
            // no value.  Format: ptr:reg (void)
            let src_reg = read_reg(state)?;
            let _ = state.get_reg(src_reg);
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::RefRelease) => {
            // cbgr_ref_release(user_ptr) → new refcount: decrement the
            // canonical rc@24 slot; at 0 the allocation is freed (the
            // AOT twin's atomicrmw-sub + dealloc-on-old==1).  Untracked
            // pointers answer 0 without touching memory.
            // Format: dst:reg, ptr:reg
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let user = state.get_reg(ptr_reg).as_integer_compatible();
            let hdr = verum_common::layout::ALLOCATION_HEADER_SIZE as usize;
            let new_count = if user > 0
                && (user as usize) >= hdr
                && state.cbgr_allocations.contains(&((user as usize) - hdr))
            {
                // SAFETY: membership gates liveness; rc@24 was written
                // by cbgr_user_allocate.
                let new = unsafe {
                    let rc_ptr = ((user as usize) - hdr
                        + verum_common::layout::ALLOCATION_HEADER_REF_COUNT_OFFSET as usize)
                        as *mut u32;
                    let new = (*rc_ptr).saturating_sub(1);
                    *rc_ptr = new;
                    new
                };
                if new == 0 {
                    super::ffi_extended::cbgr_user_deallocate(state, user);
                }
                new as i64
            } else {
                0
            };
            state.set_reg(dst, Value::from_i64(new_count));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::ValidateRef) => {
            // cbgr_validate_ref(user_ptr, expected): compare the
            // allocation header's gen@8/epoch@12 against the packed
            // `generation | (epoch << 32)` pair, field by field —
            // exactly the AOT's verum_cbgr_validate_ref (which
            // truncates each half before comparing, so junk above
            // bit 47 is ignored on both tiers).
            // Format: dst:reg, ptr:reg, expected:reg
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let exp_reg = read_reg(state)?;
            let user = state.get_reg(ptr_reg).as_integer_compatible();
            let expected = state.get_reg(exp_reg).as_integer_compatible();
            let hdr = verum_common::layout::ALLOCATION_HEADER_SIZE as usize;
            let live = if user > 0
                && (user as usize) >= hdr
                && state.cbgr_allocations.contains(&((user as usize) - hdr))
            {
                // SAFETY: membership gates liveness; the fields were
                // written by cbgr_user_allocate.
                let (actual_gen, actual_epoch) = unsafe {
                    let base = (user as usize) - hdr;
                    (
                        *((base
                            + verum_common::layout::ALLOCATION_HEADER_GENERATION_OFFSET
                                as usize) as *const u32),
                        *((base
                            + verum_common::layout::ALLOCATION_HEADER_EPOCH_OFFSET as usize)
                            as *const u16),
                    )
                };
                let expected_gen = expected as u32;
                let expected_epoch = ((expected as u64) >> 32) as u16;
                actual_gen == expected_gen && actual_epoch == expected_epoch
            } else {
                false
            };
            state.set_reg(dst, Value::from_i64(live as i64));
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
        // ================================================================
        // Allocator + public bridge (0x60-0x6A) — T0852: the arms below
        // migrated here from ffi_extended.rs (SystemSubOpcode::Cbgr*),
        // completing the Phase-4 re-homing this band was reserved for.
        // ================================================================
        Some(CbgrSubOpcode::Alloc) | Some(CbgrSubOpcode::AllocZeroed) => {
            // Both spellings allocate zero-initialised memory:
            // `cbgr_user_allocate` uses `alloc_zeroed` unconditionally so
            // uninitialised reads stay deterministic under the interpreter.
            // Zeroing the non-`_zeroed` spelling is a strengthening, never
            // an observable regression.  (The distinction still matters on
            // the `VERUM_CBGR_LEGACY_ALLOC` path, which reproduces the old
            // alloc/alloc_zeroed split exactly.)
            let zeroed = matches!(sub_op, Some(CbgrSubOpcode::AllocZeroed));
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
            // 1 GiB cap — same `verum_common::layout::MAX_ALLOCATION_SIZE`
            // ceiling as every other heap path; here cast to i64 since
            // `raw_size` arrives as i64 from the Verum register file.
            const MAX_ALLOC: i64 = verum_common::layout::MAX_ALLOCATION_SIZE as i64;
            if raw_size <= 0 || raw_size > MAX_ALLOC || raw_align <= 0 || raw_align > 4096 {
                // A rejected size/align is `Err(AllocError.InvalidSize)`
                // with the offending size as the payload field — a real
                // error value whose `.message()` works.
                let err_val = make_alloc_err(state, ALLOC_ERR_INVALID_SIZE, raw_size)?;
                state.set_reg(dst, err_val);
                return Ok(DispatchResult::Continue);
            }
            // ONE header-model authority (T0451).  `cbgr_user_allocate`
            // lays down the 32-byte `AllocationHeader` at `user - 32`,
            // registers the HEADER address in `cbgr_allocations`, and
            // records `{base_offset, total}` in the reserved word so the
            // exact `Layout` is reconstructible at free time.  Layouts
            // that overflow `isize::MAX` (adversarial bytecode requesting
            // a near-max size) return 0 rather than panicking the
            // interpreter — the same OOM-equivalent the null-pointer
            // branch modelled before.
            let legacy = cbgr_legacy_alloc();
            let ptr = if legacy {
                cbgr_legacy_allocate(state, raw_size, raw_align, zeroed)
            } else {
                cbgr_user_allocate(state, raw_size, raw_align)
            };
            if ptr == 0 {
                // Out of memory is `Err(AllocError.OutOfMemory{requested})`
                // — a real two-level variant. The ledger row that said
                // "payload stays nil because this handler does not
                // consult the type tables" is discharged: the type index
                // resolves `AllocError` in O(1).
                let err_val = make_alloc_err(state, ALLOC_ERR_OUT_OF_MEMORY, raw_size)?;
                state.set_reg(dst, err_val);
                return Ok(DispatchResult::Continue);
            }
            // Report the generation/epoch the header actually carries —
            // reading them back keeps the returned tuple and the in-memory
            // header from ever drifting apart.  The legacy path has no
            // header to read, so it keeps synthesising the old constants.
            let (generation, epoch) = if legacy {
                (1i64, state.cbgr_epoch as i64)
            } else {
                cbgr_header_generation_epoch(ptr)
            };
            // Materialise a 3-tuple matching `Pack` layout so
            // `let (ptr, g, e) = …` destructures each field at its
            // expected offset.
            let tuple_size = 3 * std::mem::size_of::<Value>();
            let tuple_obj =
                state
                    .heap
                    .alloc_with_init(crate::types::TypeId::TUPLE, tuple_size, |_data| {})?;
            let tuple_data = tuple_obj.data_ptr() as *mut Value;
            unsafe {
                std::ptr::write(tuple_data.add(0), Value::from_i64(ptr as i64));
                std::ptr::write(tuple_data.add(1), Value::from_i64(generation));
                std::ptr::write(tuple_data.add(2), Value::from_i64(epoch));
            }
            let tuple_val = Value::from_ptr(tuple_obj.as_ptr());

            // Wrap in Ok(tuple) via the canonical Result builder —
            // tag drawn from `RESULT_VARIANT_LAYOUT`, layout
            // bit-equivalent to `MakeVariant` so user code's
            // `let Ok(t) = …` destructures correctly.
            let ok_val = super::method_dispatch::make_result_variant(
                state,
                verum_common::well_known_types::result_success_tag(),
                tuple_val,
            )?;
            state.set_reg(dst, ok_val);
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::Dealloc) => {
            let _dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let _size_reg = read_reg(state)?;
            let _align_reg = read_reg(state)?;
            // T0451: this used to be an unconditional leak, justified by
            // "the interpreter has no way to match the exact Layout passed
            // at allocation time without carrying extra metadata".  With
            // the header model that justification is gone: every block is
            // self-describing (`{base_offset, total}` in the header's
            // reserved word), so the exact `Layout` IS reconstructible.
            //

            // `cbgr_user_deallocate` also sets the FREED flag and removes
            // the header address from `cbgr_allocations`, which is what
            // makes `increment_generation` and the use-after-free probes
            // observe a real transition instead of writing through a
            // header that never existed.  It is defensively a no-op on 0,
            // on untracked pointers, and on double-free (the tracked-set
            // removal is the gate), so the leak-over-double-free safety
            // property the old comment cared about is preserved.
            if !cbgr_legacy_alloc() {
                let user = state.get_reg(ptr_reg).as_integer_compatible();
                cbgr_user_deallocate(state, user);
            }
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::SecureZero) => {
            // Format: dst_ptr:reg, size:reg
            //

            // Volatile zero of `size` bytes at `dst_ptr`. In the
            // interpreter the volatile property is moot — there's no
            // optimiser pass that could elide writes — so we just
            // perform `write_volatile` on each byte to mirror the
            // ABI contract that the AOT path enforces.
            //

            // Audit: `tls-quic-security-audit spec` §2
            // Action #2.
            let dst_reg = read_reg(state)?;
            let size_reg = read_reg(state)?;
            // MEM-BULK-ADDR-DUAL-1: dual int-or-pointer extraction (see
            // CMemcpy above).
            let dst_ptr = value_as_addr(super::cbgr_helpers::resolve_arg_value(state, state.get_reg(dst_reg))) as *mut u8;
            let size_raw = state.get_reg(size_reg).as_i64();

            // SECURITY: same bounds discipline as `CMemset`.
            if size_raw < 0 || (size_raw as u64) > MAX_FFI_ALLOCATION_SIZE as u64 {
                return Err(InterpreterError::InvalidOperand {
                    message: format!(
                        "CSecureZero: size {} exceeds maximum {} or is negative",
                        size_raw, MAX_FFI_ALLOCATION_SIZE
                    ),
                });
            }
            let size = size_raw as usize;

            if !dst_ptr.is_null() && size > 0 {
                // SAFETY: size is bounded to <= MAX_FFI_ALLOCATION_SIZE and
                // dst_ptr has been null-checked. Volatile writes
                // ensure the compiler doesn't elide the loop on the
                // host side either (defence-in-depth — the
                // interpreter's runtime ABI ought to be observable
                // even though Rust optimisers shouldn't see across
                // this fn boundary).
                unsafe {
                    let mut p = dst_ptr;
                    let end = dst_ptr.add(size);
                    while p < end {
                        std::ptr::write_volatile(p, 0u8);
                        p = p.add(1);
                    }
                }
            }
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::ReallocInternal) => {
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let old_size_reg = read_reg(state)?;
            let new_size_reg = read_reg(state)?;
            let align_reg = read_reg(state)?;
            let old_ptr = state.get_reg(ptr_reg).as_integer_compatible();
            let old_size = state.get_reg(old_size_reg).as_integer_compatible();
            let new_size = state.get_reg(new_size_reg).as_integer_compatible();
            let raw_align = state.get_reg(align_reg).as_integer_compatible();
            const MAX_ALLOC: i64 = verum_common::layout::MAX_ALLOCATION_SIZE as i64;
            if new_size <= 0 || new_size > MAX_ALLOC || raw_align <= 0 || raw_align > 4096 {
                let err_val = make_alloc_err(state, ALLOC_ERR_INVALID_SIZE, new_size)?;
                state.set_reg(dst, err_val);
                return Ok(DispatchResult::Continue);
            }
            // T0451: header-model allocation, same ONE authority as
            // CbgrAlloc.  The copy is driven by the caller-supplied
            // `old_size` (not the header's) so this keeps working for an
            // old pointer that never came from the bridge — e.g. bytecode
            // that reallocs a block obtained some other way.
            let legacy = cbgr_legacy_alloc();
            let ptr = if legacy {
                cbgr_legacy_allocate(state, new_size, raw_align, false)
            } else {
                cbgr_user_allocate(state, new_size, raw_align)
            };
            if ptr == 0 {
                // The .vr contract is Result<(ptr, gen, epoch), AllocError> —
                // a bare nil desynced the caller's match (T0463), and an
                // Err(nil) payload exploded on `e.message()` (T0846).
                let err_val = make_alloc_err(state, ALLOC_ERR_OUT_OF_MEMORY, new_size)?;
                state.set_reg(dst, err_val);
                return Ok(DispatchResult::Continue);
            }
            // Preserve min(old, new) bytes from the previous block.
            if old_ptr != 0 && old_size > 0 {
                let copy = (old_size.min(new_size)) as usize;
                // SAFETY: caller-supplied old block; the copy length is
                // bounded by both the old and the fresh allocation sizes.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        old_ptr as usize as *const u8,
                        ptr as usize as *mut u8,
                        copy,
                    );
                }
            }
            // Release the old block once its contents are safe.  This is a
            // no-op unless the old pointer was itself a tracked bridge
            // allocation, so an untracked old pointer keeps the historical
            // leak-over-double-free behaviour instead of freeing memory we
            // do not own.
            if !legacy {
                cbgr_user_deallocate(state, old_ptr);
            }
            let (generation, epoch) = if legacy {
                (1i64, state.cbgr_epoch as i64)
            } else {
                cbgr_header_generation_epoch(ptr)
            };
            let tuple_size = 3 * std::mem::size_of::<Value>();
            let tuple_obj =
                state
                    .heap
                    .alloc_with_init(crate::types::TypeId::TUPLE, tuple_size, |_data| {})?;
            let tuple_data = tuple_obj.data_ptr() as *mut Value;
            unsafe {
                std::ptr::write(tuple_data.add(0), Value::from_i64(ptr as i64));
                std::ptr::write(tuple_data.add(1), Value::from_i64(generation));
                std::ptr::write(tuple_data.add(2), Value::from_i64(epoch));
            }
            let tuple_val = Value::from_ptr(tuple_obj.as_ptr());
            let ok_val = super::method_dispatch::make_result_variant(
                state,
                verum_common::well_known_types::result_success_tag(),
                tuple_val,
            )?;
            state.set_reg(dst, ok_val);
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::AllocateUser) => {
            let dst = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let align_reg = read_reg(state)?;
            let raw_size = state.get_reg(size_reg).as_integer_compatible();
            let raw_align = state.get_reg(align_reg).as_integer_compatible();
            let user = cbgr_user_allocate(state, raw_size, raw_align);
            state.set_reg(dst, Value::from_i64(user));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::DeallocUser) => {
            let _dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let user = state.get_reg(ptr_reg).as_integer_compatible();
            cbgr_user_deallocate(state, user);
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::ReallocUser) => {
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let new_size_reg = read_reg(state)?;
            let user = state.get_reg(ptr_reg).as_integer_compatible();
            let new_size = state.get_reg(new_size_reg).as_integer_compatible();
            let result = cbgr_user_realloc(state, user, new_size);
            state.set_reg(dst, Value::from_i64(result));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::ValidateBool) => {
            let dst = read_reg(state)?;
            let ref_reg = read_reg(state)?;
            let ref_val = state.get_reg(ref_reg);
            let verdict = super::cbgr::validate_ref_bool(state, ref_val);
            state.set_reg(dst, Value::from_bool(verdict));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::GetHeader) => {
            // get_header_from_ptr(user_ptr): recover the CBGR AllocationHeader
            // that precedes user data by a FIXED ALLOCATION_HEADER_SIZE bytes.
            // Format: dst:reg, ptr:reg  (exactly 2 regs — NOT PtrSub's 3).
            //
            // Mirrors `AllocationHeader.from_user_ptr` (core/mem/header.vr).
            // The offset is a fixed 32-byte constant, NOT element-scaled —
            // distinct from PtrSub (0x64), whose byte this lowering used to
            // squat (T0425).
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;

            let addr = value_as_addr(state.get_reg(ptr_reg));
            let header_addr = addr
                .checked_sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize)
                .ok_or(InterpreterError::IntegerOverflow {
                    operation: "CbgrGetHeader",
                })?;
            // Int-tagged address — same rationale as PtrSub/PtrAdd: a
            // pointer-tagged interior address becomes a droppable-looking
            // heap object and DropRef chases bytes as a header.
            state.set_reg(dst, Value::from_i64(header_addr as i64));
            Ok(DispatchResult::Continue)
        }

        Some(CbgrSubOpcode::GetGenerationUser) => {
            // cbgr_get_generation(user_ptr): *(u32*)(ptr - 32 + gen@8).
            // Format: dst:reg, ptr:reg.
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;

            let addr = value_as_addr(state.get_reg(ptr_reg));
            let gen_addr = addr
                .checked_sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize)
                .map(|h| h + verum_common::layout::ALLOCATION_HEADER_GENERATION_OFFSET as usize)
                .ok_or(InterpreterError::IntegerOverflow {
                    operation: "CbgrGetGeneration",
                })?;
            // SAFETY: the caller contract (unsafe intrinsic) requires a
            // live CBGR user pointer; the header precedes it by
            // construction of both tiers' allocators.
            let generation = unsafe { *(gen_addr as *const u32) };
            state.set_reg(dst, Value::from_i64(generation as i64));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Unimplemented sub-opcodes
        // ================================================================
        None => Err(InterpreterError::NotImplemented {
            feature: "cbgr_extended sub-opcode",
            opcode: Some(Opcode::CbgrExtended),
        }),
    }
}

// ============================================================================
// T0108 — Tier-0 typed-pointer provenance regression pins
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FunctionId;
    use crate::instruction::Instruction;
    use crate::module::{FunctionDescriptor, VbcModule};
    use std::sync::Arc;

    const REGS: u16 = 16;

    /// **End-to-end pin.**  Real bytecode, real allocator, real interpreter
    /// loop: `cbgr_allocate` (`CbgrAllocateUser`, which hands back an
    /// Int-tagged user pointer) then `DerefMut` then `Deref`, and the
    /// returned value must be the one that was written.
    ///
    /// The unit pins below register an extent by hand, which encodes an
    /// assumption about what the allocator does; this one holds the
    /// allocator and the handlers to the same story.  Pre-fix the `DerefMut`
    /// wrote nothing and the `Deref` returned the pointer, so this returns
    /// an address rather than 12345 — the assertion reads the value back,
    /// which is the only way to tell a working store from a no-op.
    ///
    /// It lives here rather than in `codegen/tests_execution.rs` (whose
    /// helpers this mirrors) because `codegen` is an optional feature and
    /// `interpreter/` is compiled under every feature set.
    #[test]
    fn end_to_end_cbgr_alloc_then_ptr_write_then_ptr_read_returns_the_written_value() {
        let mut module = VbcModule::new("t0108_end_to_end".to_string());
        let mut bytecode = Vec::new();
        for instr in &[
            Instruction::LoadI {
                dst: Reg(1),
                value: 32,
            },
            Instruction::LoadI {
                dst: Reg(2),
                value: 8,
            },
            // r0 = cbgr_allocate(32, 8) — Int-tagged user pointer.
            Instruction::CbgrExtended {
                sub_op: crate::instruction::CbgrSubOpcode::AllocateUser as u8,
                operands: vec![0, 1, 2],
            },
            Instruction::LoadI {
                dst: Reg(3),
                value: 12345,
            },
            // *r0 = 12345   (this is the store that used to vanish)
            Instruction::DerefMut {
                ref_reg: Reg(0),
                value: Reg(3),
            },
            // r4 = *r0
            Instruction::Deref {
                dst: Reg(4),
                ref_reg: Reg(0),
            },
            Instruction::Ret { value: Reg(4) },
        ] {
            crate::bytecode::encode_instruction(instr, &mut bytecode);
        }
        let bytecode_offset = module.bytecode.len() as u32;
        let bytecode_length = bytecode.len() as u32;
        module.bytecode.extend(bytecode);

        let name = module.intern_string("main");
        let mut desc = FunctionDescriptor::new(name);
        desc.register_count = REGS;
        desc.bytecode_offset = bytecode_offset;
        desc.bytecode_length = bytecode_length;
        module.add_function(desc);

        let result = crate::interpreter::Interpreter::new(Arc::new(module))
            .run_main()
            .expect("execution failed");
        assert_eq!(
            result.try_as_i64(),
            Some(12345),
            "ptr_read must return the value ptr_write stored, not the pointer",
        );
    }

    /// A state whose current frame's bytecode IS `operands`, so the handler
    /// under test decodes exactly those register bytes off the instruction
    /// stream — the real operand path, not a hand-fed shortcut.
    fn state_over_operands(operands: &[u8]) -> InterpreterState {
        let mut module = VbcModule::new("t0108_bridge_provenance".to_string());
        let offset = module.append_bytecode(operands);
        let name = module.intern_string("t0108_probe");
        let desc = FunctionDescriptor {
            id: FunctionId(0),
            name,
            bytecode_offset: offset,
            bytecode_length: operands.len() as u32,
            register_count: REGS,
            ..FunctionDescriptor::default()
        };
        module.add_function(desc);

        let mut state = InterpreterState::new(Arc::new(module));
        state.registers.push_frame(REGS);
        state
            .call_stack
            .push_frame(FunctionId(0), REGS, 0, Reg(0))
            .expect("frame");
        state.set_pc(0);
        state
    }

    /// A live bridge allocation, registered exactly the way
    /// `cbgr_user_allocate` registers one: the header address in
    /// `cbgr_allocations`, the payload extent in `cbgr_bridge_extents`.
    /// Returns the user pointer.
    ///
    /// The block is deliberately leaked for the test's lifetime — the
    /// assertions read the payload after the handler has run, so freeing it
    /// would be the use-after-free these pins exist to rule out.
    fn register_bridge_block(state: &mut InterpreterState, size: usize) -> usize {
        let hdr = verum_common::layout::ALLOCATION_HEADER_SIZE as usize;
        let layout = std::alloc::Layout::from_size_align(hdr + size, 8).unwrap();
        // SAFETY: non-zero size, valid alignment.
        let base = unsafe { std::alloc::alloc_zeroed(layout) } as usize;
        assert!(base != 0, "allocation failed");
        let user = base + hdr;
        state.cbgr_allocations.insert(base);
        state.cbgr_bridge_extents.insert(user, size);
        user
    }

    /// **The defect this task closes.**  `ptr_write` over a `cbgr_allocate`
    /// bridge block lowers to `DerefMut` with the user pointer Int-tagged.
    /// Before the bridge arm existed the handler's `if/else if` chain simply
    /// ended for that shape: nothing was written, no error was raised, and
    /// the following `ptr_read` (`Deref`) returned the POINTER — a program
    /// that stored nothing and reported success.
    #[test]
    fn ptr_write_then_ptr_read_observes_the_store_over_a_bridge_allocation() {
        // DerefMut: [ref_reg=1, value_reg=2]; Deref: [dst_reg=3, src_reg=1].
        let mut state = state_over_operands(&[1, 2, 3, 1]);
        let user = register_bridge_block(&mut state, 32);
        state.set_reg(Reg(1), Value::from_i64(user as i64));
        state.set_reg(Reg(2), Value::from_i64(12345));

        handle_deref_mut(&mut state).expect("write");

        // The store is REAL memory, observable as flat bytes — the same
        // bytes an AOT `DerefMut` of this program lays down (probed:
        // `load_byte(a)` == 57 == 12345 & 0xFF under both tiers). Pre-fix
        // these bytes stayed zero.
        // SAFETY: `user` addresses a live 32-byte payload from
        // `register_bridge_block`.
        let raw = unsafe { std::ptr::read_unaligned(user as *const u64) };
        assert_eq!(raw, 12345, "ptr_write must land flat bytes in the payload");

        handle_deref(&mut state).expect("read");
        assert_eq!(
            state.get_reg(Reg(3)).as_i64(),
            12345,
            "ptr_read must observe the store, not echo the pointer",
        );
    }

    /// The pin's own domain: `law_ptr_write_read_round_trip_over_extremes`
    /// samples the signed-64 boundary set, and every value beyond 2^47 leaves
    /// `Value`'s inline payload (`from_i64` boxes it).  Storing flat bits and
    /// rebuilding with `from_i64` round-trips all of them exactly; storing the
    /// NaN box would not have been readable back (`i64::MAX` has the same bit
    /// pattern as a box header).
    #[test]
    fn bridge_round_trip_is_exact_over_the_signed_64_extremes() {
        for v in [
            0_i64,
            1,
            -1,
            6148914691236517205,
            -6148914691236517206,
            i64::MAX,
            i64::MIN,
        ] {
            let mut state = state_over_operands(&[1, 2, 3, 1]);
            let user = register_bridge_block(&mut state, 32);
            state.set_reg(Reg(1), Value::from_i64(user as i64));
            state.set_reg(Reg(2), Value::from_i64(v));
            handle_deref_mut(&mut state).expect("write");
            handle_deref(&mut state).expect("read");
            assert_eq!(state.get_reg(Reg(3)).as_i64(), v, "round trip of {v}");
        }
    }

    /// Interior slots, not just the base pointer: `ptr_write(ptr_offset(p, i))`
    /// is the ordinary buffer idiom, and it reaches the handler as an address
    /// that is not a key of any allocation table.  The extent index is what
    /// makes it resolvable; a base-pointer-only membership test would leave
    /// every interior write a silent no-op.
    #[test]
    fn interior_bridge_slots_are_writable_and_independent() {
        // DerefMut [r1<-r2]; DerefMut [r4<-r5]; Deref [r3 <- *r4].
        let mut state = state_over_operands(&[1, 2, 4, 5, 3, 4]);
        let user = register_bridge_block(&mut state, 32);
        state.set_reg(Reg(1), Value::from_i64(user as i64));
        state.set_reg(Reg(2), Value::from_i64(111));
        state.set_reg(Reg(4), Value::from_i64((user + 8) as i64));
        state.set_reg(Reg(5), Value::from_i64(222));

        handle_deref_mut(&mut state).expect("write base");
        handle_deref_mut(&mut state).expect("write interior");
        handle_deref(&mut state).expect("read interior");

        assert_eq!(state.get_reg(Reg(3)).as_i64(), 222, "interior slot");
        // SAFETY: slot 0 lies inside the live 32-byte payload.
        let base = unsafe { std::ptr::read_unaligned(user as *const u64) };
        assert_eq!(base, 111, "the interior write must not disturb slot 0");
    }

    /// An Int-tagged value that is NOT a bridge address keeps its historical
    /// meaning — the identity fallback for plain integers/unit is load-bearing
    /// and the provenance arm must not capture it.
    #[test]
    fn plain_integers_keep_the_identity_deref() {
        let mut state = state_over_operands(&[3, 1]);
        let _ = register_bridge_block(&mut state, 32);
        state.set_reg(Reg(1), Value::from_i64(42));
        handle_deref(&mut state).expect("read");
        assert_eq!(state.get_reg(Reg(3)).as_i64(), 42);
    }

    /// A non-scalar payload has no honest packed representation: its NaN box
    /// is a tag into interpreter-private storage and the flat/boxed ambiguity
    /// is undecidable on read-back.  Reporting beats storing something the
    /// matching `ptr_read` would decode as garbage — and beats the silent
    /// no-op this replaced.
    #[test]
    fn non_scalar_bridge_store_reports_instead_of_corrupting() {
        let mut state = state_over_operands(&[1, 2]);
        let user = register_bridge_block(&mut state, 32);
        state.set_reg(Reg(1), Value::from_i64(user as i64));
        state.set_reg(Reg(2), Value::from_ptr(0x1000_usize as *mut u8));
        let err = handle_deref_mut(&mut state).expect_err("non-scalar store must report");
        assert!(
            matches!(err, InterpreterError::InvalidOperand { .. }),
            "expected InvalidOperand, got {err:?}",
        );
    }

    /// An 8-byte access whose tail leaves the block is out of bounds, not a
    /// truncated write.
    #[test]
    fn bridge_access_past_the_payload_end_reports() {
        let mut state = state_over_operands(&[1, 2]);
        let user = register_bridge_block(&mut state, 12);
        state.set_reg(Reg(1), Value::from_i64((user + 8) as i64));
        state.set_reg(Reg(2), Value::from_i64(1));
        let err = handle_deref_mut(&mut state).expect_err("must report");
        assert!(matches!(err, InterpreterError::InvalidOperand { .. }));
    }
}
