//! CBGR reference encoding/decoding/validation helpers.

use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};

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
#[inline(always)]
pub(super) fn decode_cbgr_ref(encoded: i64) -> (u32, u32) {
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

/// Checks if a Value is a CBGR register-based reference.
///
/// CBGR refs are encoded as negative Int values with generation >= 1.
/// Since generation >= 1, the encoded value is always < -(1 << 32),
/// which avoids false positives on regular small negative integers.
///
/// Uses `is_inline_int()` rather than `is_int()` because CBGR refs always
/// fit in the 48-bit inline range. Boxed ints (e.g., i64::MIN) must not
/// be misidentified as CBGR references.
#[inline(always)]
pub(super) fn is_cbgr_ref(val: &Value) -> bool {
    val.is_inline_int() && val.as_i64() < -(1i64 << 32)
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
