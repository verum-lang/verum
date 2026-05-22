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
