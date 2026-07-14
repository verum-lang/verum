//! Canonical stub-id sentinel ranges for the stdlib pre-registration
//! stages.
//!
//! The stdlib bootstrap (`verum_compiler::pipeline::stdlib_bootstrap`)
//! pre-registers name→id stubs BEFORE per-module compilation so that
//! cross-module references compile order-independently.  Each stage
//! reserves a disjoint 1M-slot sentinel band at the top of the
//! `FunctionId` space:
//!
//! | Stage | Base (`u32::MAX -`) | Contents |
//! |-------|---------------------|----------|
//! | 1 | `0x40_0000` | canonical-type static-method stubs (task #16) |
//! | 2 | `0xC0_0000` | stdlib variant-constructor stubs (task #16) |
//! | 3 | `0x100_0000` | uniquely-named public free-fn stubs (task #47) |
//! | 4 | `0x140_0000` | uniquely-named public module-const stubs (register A3-const) |
//! | 5 | `0x180_0000` | mount-miss named stubs (call-site synthesized, qualified-name remap) |
//!
//! A stub id observed at any boundary (call dispatch, archive remap,
//! global-ctor execution, metadata emission) means the PRODUCING
//! module's real body hadn't been merged when the consumer compiled —
//! consumers resolve the id to the real body BY NAME (descriptor /
//! archive-wide index), or degrade to a lenient named panic.
//!
//! This module is the single source of truth.  Every consumer that
//! previously mirrored these constants locally (interpreter ctor
//! skip, calls dispatch, archive remap tiers, stub-descriptor
//! emission, bootstrap merge-back) must go through these helpers so a
//! new stage lands everywhere at once.

/// Width of each stage's sentinel band: 1M slots.
pub const STUB_RANGE_WIDTH: u32 = 0x10_0000;

/// Stage-1 base: canonical-type static-method stubs (task #16 stage-1).
pub const STAGE1_BASE: u32 = u32::MAX - 0x40_0000;
/// Stage-2 base: stdlib variant-constructor stubs (task #16 stage-2).
pub const STAGE2_BASE: u32 = u32::MAX - 0xC0_0000;
/// Stage-3 base: uniquely-named public free-fn stubs (task #47 stage-3).
pub const STAGE3_BASE: u32 = u32::MAX - 0x100_0000;
/// Stage-4 base: uniquely-named public module-const stubs.
pub const STAGE4_BASE: u32 = u32::MAX - 0x140_0000;
/// Stage-5 base: mount-miss named stubs — an explicit braced-mount
/// item whose target module hadn't compiled yet (and whose simple
/// name is NOT globally unique, so stages 1-4 can't cover it) gets a
/// call-site-synthesized stub bound to the mount's FULL qualified
/// path; the archive name-remap chases that unambiguous spelling.
///
/// QUALIFIED-CALL-FIRST-MATCH-1 extends the band to DOTTED CALL
/// SITES: a module-shaped multi-segment call
/// (`darwin.time.monotonic_nanos()`) that resolves nowhere at compile
/// time gets a stage-5 stub bound to the call's dotted RELATIVE
/// spelling; `ArchiveBodyRemap::map_function`'s ranked
/// qualified-suffix chase resolves it (the absolute registration key
/// always ends with `.<relative spelling>` by module-anchoring
/// construction, and the user-written segment count is the ambiguity
/// floor).
pub const STAGE5_BASE: u32 = u32::MAX - 0x180_0000;

#[inline]
fn in_band(id: u32, base: u32) -> bool {
    id <= base && id >= base - STUB_RANGE_WIDTH
}

/// Stage-1 band membership.
#[inline]
pub fn in_stage1(id: u32) -> bool {
    in_band(id, STAGE1_BASE)
}

/// Stage-2 band membership.
#[inline]
pub fn in_stage2(id: u32) -> bool {
    in_band(id, STAGE2_BASE)
}

/// Stage-3 band membership.
#[inline]
pub fn in_stage3(id: u32) -> bool {
    in_band(id, STAGE3_BASE)
}

/// Stage-4 band membership.
#[inline]
pub fn in_stage4(id: u32) -> bool {
    in_band(id, STAGE4_BASE)
}

/// Stage-5 band membership.
#[inline]
pub fn in_stage5(id: u32) -> bool {
    in_band(id, STAGE5_BASE)
}

/// True when `id` lies in ANY pre-registration sentinel band.
#[inline]
pub fn is_stub_id(id: u32) -> bool {
    in_stage1(id) || in_stage2(id) || in_stage3(id) || in_stage4(id) || in_stage5(id)
}

/// True for the bands whose stubs are resolved BY NAME at finalize /
/// archive-remap time (stage-3 free fns and stage-4 consts share the
/// `emit_missing_stub_descriptors` → `ArchiveBodyRemap` name-chase).
#[inline]
pub fn is_name_resolved_stub_id(id: u32) -> bool {
    in_stage3(id) || in_stage4(id) || in_stage5(id)
}

/// XMOD call-id band membership — cross-module Call ids re-homed at
/// archive emission (XMOD-CALL-ID-BAND-1, band
/// `[XMOD_CALL_ID_BAND_BASE, XMOD_CALL_ID_BAND_BASE + 0x800_0000)`,
/// see `codegen/mod.rs` emission pass) and resolved BY NAME at
/// archive load (`ArchiveBodyRemap::map_function` Tier 0 via
/// `external_function_names`).
///
/// Deliberately NOT part of [`is_stub_id`]: a band id inside EMITTED
/// archive bytecode is the normal cross-module representation, not a
/// pre-registration stub.  The predicate exists for the resolution
/// boundary (STUB-STAGE-INSUITE-1): a band id whose exact-name
/// lookups miss at load is eligible for the same ranked
/// qualified-suffix chase as stage-5 stubs — its recorded name is a
/// module-anchored qualified spelling by the same construction — and
/// one that still survives to runtime dispatch is a load-time
/// resolution defect (`FunctionNotFound(0x2000_00xx)`), never a
/// legitimate call target.
#[inline]
pub fn in_xmod_call_band(id: u32) -> bool {
    id >= crate::module::XMOD_CALL_ID_BAND_BASE
        && id < crate::module::XMOD_CALL_ID_BAND_BASE + 0x800_0000
}

/// Which stage a stub id belongs to, if any.
#[inline]
pub fn stage_of(id: u32) -> Option<u8> {
    if in_stage1(id) {
        Some(1)
    } else if in_stage2(id) {
        Some(2)
    } else if in_stage3(id) {
        Some(3)
    } else if in_stage4(id) {
        Some(4)
    } else if in_stage5(id) {
        Some(5)
    } else {
        None
    }
}

/// Human-readable stub class for diagnostics, mirroring the lenient
/// panic wording used at the call-dispatch boundary.
pub fn stub_class(id: u32) -> Option<&'static str> {
    match stage_of(id) {
        Some(1) => Some("canonical-type static method"),
        Some(2) => Some("stdlib variant constructor"),
        Some(3) => Some("uniquely-named public free fn"),
        Some(4) => Some("uniquely-named public module const"),
        Some(5) => Some("qualified cross-module fn (mount- or dotted-call-site declared)"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_are_disjoint_and_ordered() {
        // Band tops descend stage-1 → stage-4; each band's bottom sits
        // strictly above the next band's top.
        assert!(STAGE1_BASE - STUB_RANGE_WIDTH > STAGE2_BASE);
        assert!(STAGE2_BASE - STUB_RANGE_WIDTH > STAGE3_BASE);
        assert!(STAGE3_BASE - STUB_RANGE_WIDTH > STAGE4_BASE);
        assert!(STAGE4_BASE - STUB_RANGE_WIDTH > STAGE5_BASE);
    }

    #[test]
    fn stage_of_classifies_bases_and_bottoms() {
        for (base, stage) in [
            (STAGE1_BASE, 1),
            (STAGE2_BASE, 2),
            (STAGE3_BASE, 3),
            (STAGE4_BASE, 4),
            (STAGE5_BASE, 5),
        ] {
            assert_eq!(stage_of(base), Some(stage));
            assert_eq!(stage_of(base - STUB_RANGE_WIDTH), Some(stage));
            assert!(is_stub_id(base));
        }
        assert_eq!(stage_of(0), None);
        assert_eq!(stage_of(STAGE5_BASE - STUB_RANGE_WIDTH - 1), None);
    }

    #[test]
    fn xmod_band_is_disjoint_from_stub_stages() {
        // The XMOD band lives far below every pre-registration stage
        // band and never classifies as a stub id.
        assert!(in_xmod_call_band(crate::module::XMOD_CALL_ID_BAND_BASE));
        assert!(in_xmod_call_band(0x2000_0061)); // STUB-STAGE-INSUITE-1 live victim
        assert!(!in_xmod_call_band(crate::module::XMOD_CALL_ID_BAND_BASE - 1));
        assert!(!in_xmod_call_band(
            crate::module::XMOD_CALL_ID_BAND_BASE + 0x800_0000
        ));
        assert!(!is_stub_id(crate::module::XMOD_CALL_ID_BAND_BASE));
        assert!(!in_xmod_call_band(STAGE5_BASE - STUB_RANGE_WIDTH));
    }

    #[test]
    fn pinned_absolute_values() {
        // Pinned so serialized archives keep meaning across builds:
        // these values are baked into shipped .vbca bytecode.
        assert_eq!(STAGE1_BASE, 0xFFBF_FFFF);
        assert_eq!(STAGE2_BASE, 0xFF3F_FFFF);
        assert_eq!(STAGE3_BASE, 0xFEFF_FFFF);
        assert_eq!(STAGE4_BASE, 0xFEBF_FFFF);
        assert_eq!(STAGE5_BASE, 0xFE7F_FFFF);
    }
}
