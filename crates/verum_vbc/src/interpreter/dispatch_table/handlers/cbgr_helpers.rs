//! CBGR reference encoding/decoding/validation helpers.

use super::super::super::error::{InterpreterError, InterpreterResult};
use crate::value::Value;

/// Bit 31 of the lower 32 bits encodes mutability in CBGR register references.
/// Set by RefMut (0x71), clear for Ref (0x70). Masked off during decode so
/// callers that access registers are unaffected.
pub(super) const CBGR_MUTABLE_BIT: u32 = 0x8000_0000;

/// Sentinel generation value that disables generation checks on deref.
pub(super) const CBGR_NO_CHECK_GENERATION: u32 = 0x7FFE;

/// Default epoch window size for FatRef validation.
/// Half of 16-bit range allows ~32K epoch advances before invalidation.
pub(super) const EPOCH_WINDOW_SIZE: u16 = 0x7FFF;

/// Encodes a register-based reference with CBGR generation tracking.
///

/// The encoded value is always negative, distinguishing it from regular integer values.
#[inline(always)]
pub(super) fn encode_cbgr_ref(abs_index: u32, generation: u32) -> i64 {
    -1i64 - (abs_index as i64) - ((generation as i64) << 32)
}

/// Encodes a mutable register-based reference with CBGR generation tracking.
///

/// Sets bit 31 in the lower 32 bits to mark the reference as mutable.
#[inline(always)]
pub(super) fn encode_cbgr_ref_mut(abs_index: u32, generation: u32) -> i64 {
    -1i64 - ((abs_index | CBGR_MUTABLE_BIT) as i64) - ((generation as i64) << 32)
}

/// Decodes a register-based reference into (abs_index, generation).
/// Masks off the mutability bit so register index is always correct.
///
/// Visibility raised to `pub(in super::super)` for task #18 — the
/// `do_return` stabilisation in `dispatch_table::mod.rs` reads the
/// encoded ref's `abs_index` to decide whether the ref points into
/// the current frame and therefore needs heap-cell materialisation
/// before `pop_frame` invalidates the slot.
#[inline(always)]
pub(in super::super) fn decode_cbgr_ref(encoded: i64) -> (u32, u32) {
    let raw = -(encoded + 1);
    let abs_index = ((raw & 0xFFFF_FFFF) as u32) & !CBGR_MUTABLE_BIT;
    let generation = ((raw >> 32) & 0xFFFF_FFFF) as u32;
    (abs_index, generation)
}

/// Checks if a CBGR register reference is mutable (created via RefMut).
#[inline(always)]
pub(super) fn is_cbgr_ref_mutable(encoded: i64) -> bool {
    let raw = -(encoded + 1);
    ((raw & 0xFFFF_FFFF) as u32) & CBGR_MUTABLE_BIT != 0
}

/// Strips the mutability bit from a CBGR register reference, making it immutable.
/// Used for capability downgrade (&mut -> &).
#[inline(always)]
pub(super) fn strip_cbgr_ref_mutability(encoded: i64) -> i64 {
    if is_cbgr_ref_mutable(encoded) {
        let (abs_index, generation) = decode_cbgr_ref(encoded);
        encode_cbgr_ref(abs_index, generation)
    } else {
        encoded
    }
}

/// Upper bound for a legitimately-decoded CBGR register index.
///
/// A real CBGR ref's `abs_index` is bounded by the interpreter's register
/// file size (`Registers::top`, typically 1024 with bumps).  A decoded
/// abs_index that exceeds this ceiling is essentially guaranteed to be a
/// false-positive from the negative-int range overlap (see Task #13 [A1]
/// panic: `index out of bounds: the len is 1024 but the index is
/// 1420103679`).  We pick a generous 1 << 24 = 16M slot ceiling — any
/// real interpreter that wants more registers is architectural-territory
/// (not "default-recursion-deep") and should bump the bound here in lock-
/// step with the register-file capacity.
///
/// Pin: every dispatch handler that decodes a CBGR ref via
/// `decode_cbgr_ref(val.as_i64())` MUST gate the decode through
/// `is_cbgr_ref` so the abs_index bound check fires — otherwise large
/// negative `Int` values from user code (e.g. `-10_000_000_000` or a
/// miscompiled arg-register) decode to garbage 32-bit indices and the
/// `Registers::get_absolute` debug_assert panics.  The intent of the
/// CBGR-ref encoding is "always negative" — `encode_cbgr_ref` packs
/// `(abs_index, generation)` into the low / high 32 bits.  Generation
/// starts at 1 so legitimate encoded values are always `<= -(1 << 32) -
/// 1`, but the range `(-2^47, -2^32)` overlaps with user-code negative
/// integers; the abs_index bound disambiguates.
const CBGR_REF_ABS_INDEX_MAX: u32 = 1 << 24;

/// Checks if a Value is a CBGR register-based reference.
///

/// CBGR refs are encoded as negative Int values with generation >= 1.
/// Since generation >= 1, the encoded value is always < -(1 << 32),
/// which avoids false positives on regular small negative integers.
///

/// Uses `is_inline_int()` rather than `is_int()` because CBGR refs always
/// fit in the 48-bit inline range. Boxed ints (e.g., i64::MIN) must not
/// be misidentified as CBGR references.
///

/// Tighter sanity bound (Task #13 [A1] root-cause fix): the negative-int
/// range `(-2^47, -2^32)` overlaps with legitimate user-code values like
/// `-10_000_000_000`.  Pre-fix `is_cbgr_ref` returned `true` for any such
/// value, and downstream `decode_cbgr_ref` extracted garbage abs_index
/// that crashed `Registers::get_absolute`.  The post-fix guard rejects
/// any encoded value whose decoded abs_index exceeds
/// `CBGR_REF_ABS_INDEX_MAX` — a real CBGR ref's index is always small
/// (bounded by the register-file capacity).
///
/// Visibility raised to `pub(in super::super)` for task #18 — the
/// `do_return` stabilisation in `dispatch_table::mod.rs` uses this
/// predicate to gate the register-frame escape detection.
#[inline(always)]
pub(in super::super) fn is_cbgr_ref(val: &Value) -> bool {
    if !val.is_inline_int() {
        return false;
    }
    let encoded = val.as_i64();
    if encoded >= -(1i64 << 32) {
        return false;
    }
    let (abs_index, _) = decode_cbgr_ref(encoded);
    abs_index <= CBGR_REF_ABS_INDEX_MAX
}

/// Resolve a method-call argument Value to its underlying scalar Value,
/// regardless of which reference shape the caller passed.
///
/// Verum lowers `&expr` to one of three runtime shapes depending on the
/// shape of `expr`:
///
///   * **CBGR register-ref** — emitted by `UnOp::Ref` on a bare variable
///     (`&x` where `x: Int`). Encoded as a negative inline-Int packing
///     `(abs_index, generation)`; decoded via [`decode_cbgr_ref`].
///   * **Heap-interior pointer** — emitted by `RefListElement` / `RefField`
///     when the borrow target lives inside a heap object (`&xs[i]`,
///     `&record.field`). Encoded as `Value::from_ptr(elem_ptr)` and
///     tracked in `state.cbgr_mutable_ptrs` so the generic Deref handler
///     reads through it instead of returning the pointer as a value.
///   * **ThinRef** — 16-byte CBGR reference produced by some FFI / mem
///     paths; pointer + generation + epoch in heap. Auto-derefed via
///     `*const Value` when non-null.
///
/// Every primitive-method intercept that consumes `other: &T` MUST funnel
/// the argument through this helper. The buggy alternative — checking
/// only `is_cbgr_ref` and otherwise calling `val.as_i64()` — silently
/// returns the raw pointer address when the caller borrowed a list
/// element or a field, producing nonsense comparisons (e.g. for
/// `xs[0].cmp(&xs[1])` the bubble-sort decision flips).
///
/// The helper deliberately does NOT validate generation/epoch — it
/// preserves whatever the caller's CBGR-validation policy is (see
/// `state.config.cbgr_enabled`). Callers that need validation should
/// invoke [`validate_cbgr_generation`] on the decoded `(abs_index, gen)`
/// pair separately; matches the pre-existing behaviour of the 24 sites
/// this helper replaces.
#[inline]
pub(super) fn resolve_arg_value(
    state: &super::super::super::state::InterpreterState,
    val: Value,
) -> Value {
    if is_cbgr_ref(&val) {
        let (abs_index, _gen) = decode_cbgr_ref(val.as_i64());
        return state.registers.get_absolute(abs_index);
    }
    if val.is_ptr() && !val.is_nil() {
        let ptr_addr = val.as_ptr::<u8>() as usize;
        if state.cbgr_mutable_ptrs.contains(&ptr_addr) {
            return unsafe { *(ptr_addr as *const Value) };
        }
    }
    if val.is_thin_ref() {
        let thin_ref = val.as_thin_ref();
        if !thin_ref.ptr.is_null() {
            return unsafe { *(thin_ref.ptr as *const Value) };
        }
    }
    val
}

/// Write `value` through whatever reference shape `slot_val` holds, mutating
/// the ORIGIN storage the reference points at. Write-side twin of
/// [`resolve_arg_value`] — the same three reference shapes, resolved for a
/// store instead of a load:
///
///   * **CBGR register-ref** → `Registers::set_absolute(abs_index, value)`.
///     A CBGR ref encodes the ABSOLUTE origin register index and is copied
///     verbatim across call frames, so a single resolution reaches the true
///     origin no matter how many wrapper frames the `&mut` passed through
///     (`safe_getsockname(addr_len: &mut UInt32) { getsockname(…, addr_len) }`
///     — the ref in `addr_len`'s slot still points at the caller's variable).
///   * **Heap-interior pointer** (tracked in `cbgr_mutable_ptrs`) → write the
///     `Value` at the interior address (`&record.field` / `&xs[i]`).
///   * **ThinRef** → write the `Value` at the ref's heap pointer.
///
/// Returns `true` if the value was written through a reference; `false` if
/// `slot_val` is not a reference (the caller should then store into the local
/// register directly — the same-frame `&mut local` case where the source
/// register already IS the origin scalar slot).
///
/// FFI-WRITEBACK-THRU-REF-1 (#25): the previous write-back unconditionally
/// did `set_reg(source_reg)`, which for a `&mut` OUT-param passed through a
/// wrapper frame overwrote the REFERENCE in the wrapper's parameter slot with
/// the returned scalar and never touched the caller's variable — so a C
/// OUT-param (`socklen_t*`) write was silently dropped one frame up. Following
/// the ref to its origin makes the C write land on the caller's storage.
#[inline]
// Only the `ffi`-gated FFI writeback loops call this; it is genuinely dead
// (and correctly warns) in a build without the `ffi` feature.
#[cfg_attr(not(feature = "ffi"), allow(dead_code))]
pub(in super::super) fn write_through_ref(
    state: &mut super::super::super::state::InterpreterState,
    slot_val: Value,
    value: Value,
) -> bool {
    if is_cbgr_ref(&slot_val) {
        let (abs_index, _gen) = decode_cbgr_ref(slot_val.as_i64());
        state.registers.set_absolute(abs_index, value);
        return true;
    }
    if slot_val.is_ptr() && !slot_val.is_nil() {
        let ptr_addr = slot_val.as_ptr::<u8>() as usize;
        if state.cbgr_mutable_ptrs.contains(&ptr_addr) {
            unsafe { *(ptr_addr as *mut Value) = value };
            return true;
        }
    }
    if slot_val.is_thin_ref() {
        let thin_ref = slot_val.as_thin_ref();
        if !thin_ref.ptr.is_null() {
            unsafe { *(thin_ref.ptr as *mut Value) = value };
            return true;
        }
    }
    false
}

/// FFI-WRITEBACK-THRU-REF-1 (#25): normalize `&mut` scalar OUT-param args
/// that arrive as bare mutable references — a wrapper's `&mut` parameter
/// forwarded straight to an FFI call (`safe_getsockname(addr_len: &mut
/// UInt32) { getsockname(…, addr_len) }`), as opposed to an explicit
/// `&mut expr` the codegen already stripped and tagged in `source_reg_map`.
///
/// For each such argument this both:
///   * **derefs** it to the referent scalar, so the value marshalled into
///     the C call is the real number (128) — the raw CBGR-ref bits would
///     otherwise marshal as a garbage `socklen_t` input; and
///   * **registers** its source register in `source_reg_map`, so the
///     post-call write-back fires and — via [`write_through_ref`] — lands
///     the C store on the caller's origin variable one (or more) frames up.
///
/// Immutable references are deliberately left untouched: they are never
/// write-back targets, and following one would corrupt the origin with the
/// unmodified temp value. An explicit `&mut x` on a same-frame local is
/// already reduced to a plain scalar by the codegen, so it does not match
/// here and keeps its existing same-frame write-back path.
pub(in super::super) fn normalize_ffi_ref_args(
    state: &super::super::super::state::InterpreterState,
    arg_regs: &[crate::instruction::Reg],
    args: &mut [Value],
    source_reg_map: &mut std::collections::HashMap<u8, u16>,
) {
    for (i, arg_reg) in arg_regs.iter().enumerate() {
        if i >= args.len() {
            break;
        }
        let v = args[i];
        let is_mut_ref = if is_cbgr_ref(&v) {
            is_cbgr_ref_mutable(v.as_i64())
        } else if v.is_ptr() && !v.is_nil() {
            state
                .cbgr_mutable_ptrs
                .contains(&(v.as_ptr::<u8>() as usize))
        } else {
            v.is_thin_ref()
        };
        if is_mut_ref {
            args[i] = resolve_arg_value(state, v);
            source_reg_map.entry(i as u8).or_insert(arg_reg.0);
        }
    }
}

/// Validates CBGR generation and epoch for a register-based reference.
///

/// Returns `Ok(())` if the reference is valid, or `Err(Panic)` if
/// use-after-free is detected (generation or epoch mismatch).
///

/// The epoch check prevents the ABA problem where generation wraps around
/// to a previously-used value after 2^32 allocations.
#[inline(always)]
pub(super) fn validate_cbgr_generation(
    state: &super::super::super::state::InterpreterState,
    abs_index: u32,
    ref_generation: u32,
) -> InterpreterResult<()> {
    use super::super::super::registers::GEN_NO_CHECK;

    // Honour `InterpreterConfig.cbgr_enabled`: when the caller
    // has explicitly disabled CBGR validation (e.g. for a
    // max-throughput build that ran the static escape analyzer
    // upfront and proved every reference safe), the dispatch
    // loop must skip the per-deref generation check. This
    // short-circuit closes the inert-defense pattern: prior to
    // wiring, the documented opt-out had no observable effect —
    // every dereference paid the ~15 ns validation cost
    // regardless of caller intent.
    if !state.config.cbgr_enabled {
        return Ok(());
    }

    if ref_generation != CBGR_NO_CHECK_GENERATION && ref_generation != GEN_NO_CHECK {
        let current_gen = state.registers.get_generation(abs_index);
        if ref_generation != current_gen {
            return Err(InterpreterError::Panic {
                message: format!(
                    "CBGR use-after-free detected: expected generation {}, found {}",
                    ref_generation, current_gen
                ),
            });
        }
    }
    Ok(())
}

/// Validates epoch using window comparison to handle 16-bit truncation.
///

/// The 64-bit global epoch is truncated to 16-bit when stored in references
/// (ThinRef, FatRef). This creates a potential ABA problem where the epoch
/// wraps every 65536 operations.
///

/// Window validation solves this by checking if the reference epoch is within
/// a valid window of the current epoch. References are considered valid if:
/// 1. The truncated epochs match exactly, OR
/// 2. The reference epoch is within the valid window (accounting for wraparound)
///

/// The window size is configurable but defaults to 32768 (half of 16-bit range),
/// allowing references to remain valid for ~32K epoch advances.
#[inline(always)]
pub(super) fn validate_epoch_window(ref_epoch: u16, current_epoch: u64, window_size: u16) -> bool {
    let current_epoch_16 = (current_epoch & 0xFFFF) as u16;

    // Direct match
    if ref_epoch == current_epoch_16 {
        return true;
    }

    // Window comparison: reference must be within [current - window, current]
    // Use wrapping arithmetic to handle 16-bit overflow
    let diff = current_epoch_16.wrapping_sub(ref_epoch);
    diff <= window_size
}
