//! CBGR reference encoding/decoding/validation helpers.
//!
//! **SOLE authority** for the interpreter's Tier-0 register-reference encoding.
//! Every dispatch handler that creates, classifies, or decodes a register-ref
//! funnels through `encode_cbgr_ref{,_mut}` / `is_cbgr_ref` / `decode_cbgr_ref`
//! / `is_cbgr_ref_mutable` / `strip_cbgr_ref_mutability` here.
//!
//! # T0367 / CBGR-REGREF-TAG-1 — the soundness fix
//!
//! Register-refs used to be UNTAGGED negative inline `Int`s
//! (`-1 - abs - (gen << 32)`), squatting inside `TAG_INTEGER` space. A legal
//! user `Int` of the form `-(k·2^32 + m)` with `m ≤ 2^24` therefore
//! false-positived [`is_cbgr_ref`], decoded to a garbage absolute register
//! index, and hit `Registers::get_absolute` — a loud ICE in debug and a SILENT
//! substitution of a random register's value for the user's `Int` in release.
//! The `1 << 24` abs-index bound shrank the collision window but never closed
//! it — refs stayed forgeable from data, breaking the three-tier CBGR premise.
//!
//! The encoding now lives in a DISCRIMINATED tag (`Value::cbgr_regref`, under
//! `TAG_UNIT` with a marker bit — see `value.rs`), disjoint from `TAG_INTEGER`,
//! so `is_cbgr_ref` is an EXACT tag test and a user `Int` can never collide.
//!
//! `VERUM_CBGR_LEGACY_INT_REFS=1` restores the pre-T0367 int-space encoding
//! (with its soundness hole) for A/B differential comparison ONLY.

use super::super::super::error::{InterpreterError, InterpreterResult};
use crate::value::Value;
use std::sync::OnceLock;

/// Sentinel generation value that disables generation checks on deref.
/// Fits in the tagged encoding's 22-bit generation field.
pub(super) const CBGR_NO_CHECK_GENERATION: u32 = 0x7FFE;

/// Default epoch window size for FatRef validation.
/// Half of 16-bit range allows ~32K epoch advances before invalidation.
pub(super) const EPOCH_WINDOW_SIZE: u16 = 0x7FFF;

/// Width of the generation field carried by a tagged register-ref (bits).
/// The slot generation is a full 32-bit counter (`registers::GEN_MAX`); the ref
/// carries its low 22 bits and [`validate_cbgr_generation`] compares modulo
/// 2^22 (the ThinRef 16-bit epoch-window precedent).
const CBGR_REGREF_GEN_BITS: u32 = 22;
/// Mask for the low `CBGR_REGREF_GEN_BITS` of a generation counter.
const CBGR_REGREF_GEN_MASK: u32 = (1u32 << CBGR_REGREF_GEN_BITS) - 1;

// --- Kill-switch: legacy int-space encoding (A/B only) ----------------------

/// `VERUM_CBGR_LEGACY_INT_REFS=1` (or `true`) restores the pre-T0367
/// negative-inline-`Int` register-ref encoding. Read once and cached. Default
/// off. Exists solely so a differential run can confirm the tagged encoding is
/// what closed the T0367 soundness hole.
#[inline(always)]
fn legacy_int_refs() -> bool {
    static CELL: OnceLock<bool> = OnceLock::new();
    *CELL.get_or_init(|| {
        std::env::var_os("VERUM_CBGR_LEGACY_INT_REFS")
            .map(|v| v == "1" || v == "true")
            .unwrap_or(false)
    })
}

/// Legacy mutability bit (bit 31 of the abs word) — legacy encoding only.
const LEGACY_CBGR_MUTABLE_BIT: u32 = 0x8000_0000;
/// Legacy abs-index sanity ceiling (the `1 << 24` window that never closed the
/// hole) — legacy `is_cbgr_ref` only.
const LEGACY_CBGR_REF_ABS_INDEX_MAX: u32 = 1 << 24;

#[inline(always)]
fn legacy_encode(abs_index: u32, generation: u32) -> i64 {
    -1i64 - (abs_index as i64) - ((generation as i64) << 32)
}

#[inline(always)]
fn legacy_decode(encoded: i64) -> (u32, u32) {
    let raw = -(encoded + 1);
    let abs_index = ((raw & 0xFFFF_FFFF) as u32) & !LEGACY_CBGR_MUTABLE_BIT;
    let generation = ((raw >> 32) & 0xFFFF_FFFF) as u32;
    (abs_index, generation)
}

// --- Encode / decode / classify (SOLE authority) ----------------------------

/// Encodes an immutable register-based reference with CBGR generation tracking.
///
/// Returns a discriminated tagged `Value` (never an `Int`) — see the module
/// docs. Under the kill-switch it returns the legacy negative-`Int` form.
#[inline(always)]
pub(super) fn encode_cbgr_ref(abs_index: u32, generation: u32) -> Value {
    if legacy_int_refs() {
        Value::from_i64(legacy_encode(abs_index, generation))
    } else {
        Value::cbgr_regref(abs_index, generation, false)
    }
}

/// Encodes a mutable (`&mut`) register-based reference.
#[inline(always)]
pub(super) fn encode_cbgr_ref_mut(abs_index: u32, generation: u32) -> Value {
    if legacy_int_refs() {
        Value::from_i64(legacy_encode(abs_index | LEGACY_CBGR_MUTABLE_BIT, generation))
    } else {
        Value::cbgr_regref(abs_index, generation, true)
    }
}

/// Decodes a register-based reference into `(abs_index, generation)`.
///
/// PRECONDITION: `encoded` is a register-ref ([`is_cbgr_ref`] returned true) —
/// every call site gates on that (the pinned contract). In the tagged form the
/// generation is the low 22 bits of the slot counter.
///
/// Visibility is `pub(in crate::interpreter)` so the collection-normaliser in
/// `interpreter::state` routes through this SOLE authority (T0367 deleted its
/// inlined copy) and so the `do_return` frame-escape stabilisation in
/// `dispatch_table::mod.rs` can read the ref's `abs_index`.
#[inline(always)]
pub(in crate::interpreter) fn decode_cbgr_ref(encoded: Value) -> (u32, u32) {
    if legacy_int_refs() {
        legacy_decode(encoded.as_i64())
    } else {
        (encoded.cbgr_regref_abs(), encoded.cbgr_regref_generation())
    }
}

/// Checks if a CBGR register reference is mutable (created via RefMut).
///
/// PRECONDITION: `encoded` is a register-ref ([`is_cbgr_ref`] returned true).
#[inline(always)]
pub(super) fn is_cbgr_ref_mutable(encoded: Value) -> bool {
    if legacy_int_refs() {
        let raw = -(encoded.as_i64() + 1);
        ((raw & 0xFFFF_FFFF) as u32) & LEGACY_CBGR_MUTABLE_BIT != 0
    } else {
        encoded.cbgr_regref_is_mut()
    }
}

/// Strips the mutability bit from a CBGR register reference, making it immutable.
/// Used for capability downgrade (`&mut` -> `&`).
#[inline(always)]
pub(super) fn strip_cbgr_ref_mutability(encoded: Value) -> Value {
    if is_cbgr_ref_mutable(encoded) {
        let (abs_index, generation) = decode_cbgr_ref(encoded);
        encode_cbgr_ref(abs_index, generation)
    } else {
        encoded
    }
}

/// Checks if a Value is a CBGR register-based reference.
///
/// EXACT tag test (T0367): a register-ref is `Value::cbgr_regref` — `TAG_UNIT`
/// with the register-ref marker bit set — which is disjoint from every `Int`,
/// so no user integer, of any value or sign, can be misclassified. There is no
/// arithmetic-range guessing and no residual collision window.
///
/// Under `VERUM_CBGR_LEGACY_INT_REFS=1` this falls back to the pre-T0367
/// negative-`Int` range guess (which carries the soundness hole BY DESIGN, for
/// A/B comparison only).
///
/// Visibility is `pub(in crate::interpreter)` for the `do_return` frame-escape
/// gate in `dispatch_table::mod.rs` and the collection normaliser in
/// `interpreter::state`.
#[inline(always)]
pub(in crate::interpreter) fn is_cbgr_ref(val: &Value) -> bool {
    if legacy_int_refs() {
        if !val.is_inline_int() {
            return false;
        }
        let encoded = val.as_i64();
        if encoded >= -(1i64 << 32) {
            return false;
        }
        let (abs_index, _) = legacy_decode(encoded);
        abs_index <= LEGACY_CBGR_REF_ABS_INDEX_MAX
    } else {
        val.is_cbgr_regref()
    }
}

/// Resolve a method-call argument Value to its underlying scalar Value,
/// regardless of which reference shape the caller passed.
///
/// Verum lowers `&expr` to one of three runtime shapes depending on the
/// shape of `expr`:
///
///   * **CBGR register-ref** — emitted by `UnOp::Ref` on a bare variable
///     (`&x` where `x: Int`). A discriminated tagged `Value` (T0367,
///     `Value::cbgr_regref`) packing `(abs_index, generation, mutable)`;
///     classified by [`is_cbgr_ref`] and decoded via [`decode_cbgr_ref`].
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
        let (abs_index, _gen) = decode_cbgr_ref(val);
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
        let (abs_index, _gen) = decode_cbgr_ref(slot_val);
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
            is_cbgr_ref_mutable(v)
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

/// True iff a decoded register-ref generation matches the live slot generation.
///
/// SOLE authority for register-ref generation equality. A tagged register-ref
/// carries only the low `CBGR_REGREF_GEN_BITS` (22) of the 32-bit slot counter
/// (`registers::GEN_MAX`), so the comparison is modulo 2^22 — the 16-bit
/// `ThinRef` epoch-window precedent. Without the mask, a ref into any slot
/// whose generation has passed 2^22 would spuriously read as stale. Used by the
/// deref validator ([`validate_cbgr_generation`]) and the `is_valid` /
/// `validate_epoch` register-ref introspection intrinsics, so all three agree.
///
/// The sentinels (`CBGR_NO_CHECK_GENERATION`, `GEN_NO_CHECK` = 0x7FFE) fit in
/// 22 bits and are handled by the callers before reaching this comparison.
#[inline(always)]
pub(super) fn regref_generation_matches(ref_generation: u32, current_generation: u32) -> bool {
    (ref_generation & CBGR_REGREF_GEN_MASK) == (current_generation & CBGR_REGREF_GEN_MASK)
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
        // Modulo-2^22 comparison via the SOLE authority (see
        // `regref_generation_matches`): the tagged register-ref carries only 22
        // generation bits. Short-lived refs — the overwhelming majority —
        // capture and deref within one unchanged generation, so this is exact in
        // practice; divergence would need a slot bumped an exact multiple of
        // 2^22 times between borrow and use.
        if !regref_generation_matches(ref_generation, current_gen) {
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

#[cfg(test)]
mod tests {
    //! Authority-level tests for the T0367 tagged register-ref encoding.
    //! These run in the DEFAULT (tagged) mode — the kill-switch is off unless
    //! `VERUM_CBGR_LEGACY_INT_REFS` is set in the environment.
    use super::*;

    /// The soundness property at the authority boundary: no user `Int` — of any
    /// value or sign — is classified as a register-ref. This is the exact
    /// function (and the exact seed value) from the T0367 acceptance.
    #[test]
    fn is_cbgr_ref_rejects_user_ints() {
        assert!(!is_cbgr_ref(&Value::from_i64(-131008744790)));
        // The full collision family -(k·2^32 + m) - 1 the legacy encoding
        // decoded to a small abs-index, plus assorted user ints and both
        // inline boundaries.
        for k in 0..64i64 {
            for m in [0i64, 1, 2, 1647571, 12242261, (1 << 24) - 1] {
                let bad = -(k * (1i64 << 32) + m) - 1;
                assert!(
                    !is_cbgr_ref(&Value::from_i64(bad)),
                    "user Int {} (k={}, m={}) misclassified as a register-ref",
                    bad,
                    k,
                    m
                );
            }
        }
        for &i in &[0i64, 1, -1, i64::MIN, i64::MAX, -4296614867] {
            assert!(!is_cbgr_ref(&Value::from_i64(i)), "user Int {} misread", i);
        }
        // Other Value kinds are not register-refs either.
        assert!(!is_cbgr_ref(&Value::unit()));
        assert!(!is_cbgr_ref(&Value::from_bool(true)));
        assert!(!is_cbgr_ref(&Value::from_f64(1.5)));
    }

    /// A genuinely-encoded register-ref round-trips through the authority:
    /// encode → is_cbgr_ref true → decode → same (abs, gen); mutability
    /// preserved and strippable.
    #[test]
    fn encoded_regref_roundtrips() {
        let cases = [
            (0u32, 1u32, false),
            (7, 3, true),
            (1023, CBGR_NO_CHECK_GENERATION, false),
            (12242261, 42, true), // the T0367 abs — a *legitimate* ref here
            ((1 << 24) - 1, (1 << 22) - 1, true),
        ];
        for (abs, generation, mutable) in cases {
            let enc = if mutable {
                encode_cbgr_ref_mut(abs, generation)
            } else {
                encode_cbgr_ref(abs, generation)
            };
            assert!(is_cbgr_ref(&enc), "encoded regref not classified (abs={})", abs);
            assert!(!enc.is_int(), "a tagged regref must not be an Int");
            let (dabs, dgen) = decode_cbgr_ref(enc);
            assert_eq!(dabs, abs, "abs round-trip");
            assert_eq!(dgen, generation & ((1 << 22) - 1), "gen round-trip mod 2^22");
            assert_eq!(is_cbgr_ref_mutable(enc), mutable, "mutability round-trip");

            let shared = strip_cbgr_ref_mutability(enc);
            assert!(is_cbgr_ref(&shared));
            assert!(!is_cbgr_ref_mutable(shared), "strip must clear mutability");
            let (sabs, sgen) = decode_cbgr_ref(shared);
            assert_eq!((sabs, sgen), (abs, generation & ((1 << 22) - 1)));
        }
    }
}
