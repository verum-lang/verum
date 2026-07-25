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
const fn in_band(id: u32, base: u32) -> bool {
    id <= base && id >= base - STUB_RANGE_WIDTH
}

/// Stage-1 band membership.
#[inline]
pub const fn in_stage1(id: u32) -> bool {
    in_band(id, STAGE1_BASE)
}

/// Stage-2 band membership.
#[inline]
pub const fn in_stage2(id: u32) -> bool {
    in_band(id, STAGE2_BASE)
}

/// Stage-3 band membership.
#[inline]
pub const fn in_stage3(id: u32) -> bool {
    in_band(id, STAGE3_BASE)
}

/// Stage-4 band membership.
#[inline]
pub const fn in_stage4(id: u32) -> bool {
    in_band(id, STAGE4_BASE)
}

/// Stage-5 band membership.
#[inline]
pub const fn in_stage5(id: u32) -> bool {
    in_band(id, STAGE5_BASE)
}

/// True when `id` lies in ANY pre-registration sentinel band.
#[inline]
pub const fn is_stub_id(id: u32) -> bool {
    in_stage1(id) || in_stage2(id) || in_stage3(id) || in_stage4(id) || in_stage5(id)
}

/// True for the bands whose stubs are resolved BY NAME at finalize /
/// archive-remap time (stage-3 free fns and stage-4 consts share the
/// `emit_missing_stub_descriptors` → `ArchiveBodyRemap` name-chase).
#[inline]
pub const fn is_name_resolved_stub_id(id: u32) -> bool {
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
pub const fn in_xmod_call_band(id: u32) -> bool {
    id >= crate::module::XMOD_CALL_ID_BAND_BASE
        && id < crate::module::XMOD_CALL_ID_BAND_BASE + 0x800_0000
}

/// FRESH user-band base — merge-time `band_redirect` targets for
/// cross-archive names unresolvable at merge order
/// (`codegen/mod.rs::user_xmod_band_next` starts here).  Distinct
/// from the emission band above: fresh ids are minted during MERGE,
/// carried with their names via `user_xmod_carry`, and re-homed into
/// the emission band by `build_module`'s external pass.
pub const XMOD_FRESH_BAND_BASE: u32 = crate::module::XMOD_CALL_ID_BAND_BASE + 0x1000_0000;

/// Width of the fresh user band (`[0x3000_0000, 0x3800_0000)`).
/// Kept below the `EXTERN_SENTINEL_THRESHOLD` (`u32::MAX / 4 =`
/// `0x3FFF_FFFF`) gate that `build_module`'s external detection
/// relies on — pinned by `xmod_ranges_disjoint_and_bounded`.
pub const XMOD_FRESH_BAND_WIDTH: u32 = 0x0800_0000;

/// **REMAP-POISON-1 (T0144)** — the single sentinel
/// `ArchiveBodyRemap::map_function` substitutes for an ORDINARY-range
/// archive-local id that reached the Tier-3 fallback with no remap
/// entry and no name anywhere (its producing sibling was pruned from
/// the merge set). Identity-passing such an id would land the call on
/// whatever unrelated user function occupies the raw number — the
/// silent-misroute class this task exists to kill. The poison value
/// sits at the top of the fresh band (never minted by the allocator:
/// the exhaustion assert fires a full slot earlier), is recognised by
/// `is_xmod_name_reference` (band member), and dies LOUD at dispatch
/// with a dedicated diagnostic instead of calling garbage.
pub const REMAP_POISON_ID: u32 = XMOD_FRESH_BAND_BASE + XMOD_FRESH_BAND_WIDTH - 1;

/// Recognise the remap poison sentinel.
#[inline]
pub const fn is_remap_poison(id: u32) -> bool {
    id == REMAP_POISON_ID
}

/// Fresh-band membership (XMOD-FRESH-BAND-WINDOW-1).
#[inline]
pub const fn in_xmod_fresh_band(id: u32) -> bool {
    id >= XMOD_FRESH_BAND_BASE && id < XMOD_FRESH_BAND_BASE + XMOD_FRESH_BAND_WIDTH
}

/// The ONE "is this function id a BY-NAME cross-module reference?"
/// predicate (XMOD-FRESH-BAND-WINDOW-1).  Pre-fix, consumers gated on
/// `in_xmod_call_band` alone: a FRESH id (0x3000_0000+) was
/// recognized as a band NOWHERE until `build_module` re-homed it —
/// any path that saw one before/without re-home (a future
/// EXTERN_SENTINEL/filter change, a diagnostic, an AOT chase) fell
/// through to "not found" instead of chasing the carried name.  Every
/// resolution boundary should gate on THIS predicate, not on the
/// individual windows.
#[inline]
pub const fn is_xmod_name_reference(id: u32) -> bool {
    in_xmod_call_band(id) || in_xmod_fresh_band(id)
}

/// Which stage a stub id belongs to, if any.
#[inline]
pub const fn stage_of(id: u32) -> Option<u8> {
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
pub const fn stub_class(id: u32) -> Option<&'static str> {
    match stage_of(id) {
        Some(1) => Some("canonical-type static method"),
        Some(2) => Some("stdlib variant constructor"),
        Some(3) => Some("uniquely-named public free fn"),
        Some(4) => Some("uniquely-named public module const"),
        Some(5) => Some("qualified cross-module fn (mount- or dotted-call-site declared)"),
        _ => None,
    }
}

/// Band tops descend stage-1 → stage-5 and each band's bottom sits strictly
/// above the next band's top, so the stub ranges never overlap.
///
/// Enforced at COMPILE time rather than by a test: the bases and the width are
/// `const`, so a base edit that collapses two bands fails the build — an
/// overlap would otherwise misclassify stub ids at runtime.
const _: () = {
    assert!(STAGE1_BASE - STUB_RANGE_WIDTH > STAGE2_BASE);
    assert!(STAGE2_BASE - STUB_RANGE_WIDTH > STAGE3_BASE);
    assert!(STAGE3_BASE - STUB_RANGE_WIDTH > STAGE4_BASE);
    assert!(STAGE4_BASE - STUB_RANGE_WIDTH > STAGE5_BASE);
};

/// Stage classification is total and exact: each base and each band bottom
/// classifies to its own stage, and ids outside every band classify to none.
const _: () = {
    assert!(matches!(stage_of(STAGE1_BASE), Some(1)));
    assert!(matches!(stage_of(STAGE1_BASE - STUB_RANGE_WIDTH), Some(1)));
    assert!(matches!(stage_of(STAGE2_BASE), Some(2)));
    assert!(matches!(stage_of(STAGE2_BASE - STUB_RANGE_WIDTH), Some(2)));
    assert!(matches!(stage_of(STAGE3_BASE), Some(3)));
    assert!(matches!(stage_of(STAGE3_BASE - STUB_RANGE_WIDTH), Some(3)));
    assert!(matches!(stage_of(STAGE4_BASE), Some(4)));
    assert!(matches!(stage_of(STAGE4_BASE - STUB_RANGE_WIDTH), Some(4)));
    assert!(matches!(stage_of(STAGE5_BASE), Some(5)));
    assert!(matches!(stage_of(STAGE5_BASE - STUB_RANGE_WIDTH), Some(5)));

    assert!(is_stub_id(STAGE1_BASE));
    assert!(is_stub_id(STAGE2_BASE));
    assert!(is_stub_id(STAGE3_BASE));
    assert!(is_stub_id(STAGE4_BASE));
    assert!(is_stub_id(STAGE5_BASE));

    assert!(stage_of(0).is_none());
    assert!(stage_of(STAGE5_BASE - STUB_RANGE_WIDTH - 1).is_none());
};

/// REMAP-POISON-1 (T0144) pins.
const _: () = {
    // 1. The poison sentinel occupies the fresh band's TOP slot —
    //    recognised by the band predicates (so serialization / external
    //    detection treat it like a name reference and it can never
    //    silently escape the discipline)...
    assert!(is_remap_poison(REMAP_POISON_ID));
    assert!(in_xmod_fresh_band(REMAP_POISON_ID));
    assert!(is_xmod_name_reference(REMAP_POISON_ID));
    assert!(!is_stub_id(REMAP_POISON_ID));
    // 2. ...and exactly ONE value is poison.
    assert!(!is_remap_poison(REMAP_POISON_ID - 1));
    assert!(!is_remap_poison(REMAP_POISON_ID + 1));
    // 3. Below the extern-sentinel gate, like the rest of the band.
    assert!((REMAP_POISON_ID as u64) < (u32::MAX / 4) as u64);
};

/// XMOD-FRESH-BAND-WINDOW-1 pins.
const _: () = {
    // 1. Fresh band sits strictly ABOVE the emission band (no overlap).
    assert!(XMOD_FRESH_BAND_BASE == crate::module::XMOD_CALL_ID_BAND_BASE + 0x1000_0000);
    assert!(XMOD_FRESH_BAND_BASE >= crate::module::XMOD_CALL_ID_BAND_BASE + 0x800_0000);
    // 2. Fresh band stays BELOW the EXTERN_SENTINEL_THRESHOLD gate
    //    (u32::MAX / 4) that build_module's external detection uses — a
    //    fresh id past that gate would silently skip re-homing.
    assert!(XMOD_FRESH_BAND_BASE + XMOD_FRESH_BAND_WIDTH <= u32::MAX / 4);
    // 3. Fresh band never classifies as a stub stage.
    assert!(!is_stub_id(XMOD_FRESH_BAND_BASE));
    assert!(!is_stub_id(XMOD_FRESH_BAND_BASE + XMOD_FRESH_BAND_WIDTH - 1));
    // 4. The combined name-reference predicate covers BOTH windows and
    //    nothing between/around them.
    assert!(is_xmod_name_reference(crate::module::XMOD_CALL_ID_BAND_BASE));
    assert!(is_xmod_name_reference(XMOD_FRESH_BAND_BASE));
    assert!(is_xmod_name_reference(
        XMOD_FRESH_BAND_BASE + XMOD_FRESH_BAND_WIDTH - 1
    ));
    // the gap between the windows
    assert!(!is_xmod_name_reference(
        crate::module::XMOD_CALL_ID_BAND_BASE + 0x800_0000
    ));
    assert!(!is_xmod_name_reference(XMOD_FRESH_BAND_BASE - 1));
    assert!(!is_xmod_name_reference(
        XMOD_FRESH_BAND_BASE + XMOD_FRESH_BAND_WIDTH
    ));
};

/// The XMOD band lives far below every pre-registration stage band and
/// never classifies as a stub id.
const _: () = {
    assert!(in_xmod_call_band(crate::module::XMOD_CALL_ID_BAND_BASE));
    assert!(in_xmod_call_band(0x2000_0061)); // STUB-STAGE-INSUITE-1 live victim
    assert!(!in_xmod_call_band(
        crate::module::XMOD_CALL_ID_BAND_BASE - 1
    ));
    assert!(!in_xmod_call_band(
        crate::module::XMOD_CALL_ID_BAND_BASE + 0x800_0000
    ));
    assert!(!is_stub_id(crate::module::XMOD_CALL_ID_BAND_BASE));
    assert!(!in_xmod_call_band(STAGE5_BASE - STUB_RANGE_WIDTH));
};

/// Pinned so serialized archives keep meaning across builds: these values
/// are baked into shipped .vbca bytecode.
const _: () = {
    assert!(STAGE1_BASE == 0xFFBF_FFFF);
    assert!(STAGE2_BASE == 0xFF3F_FFFF);
    assert!(STAGE3_BASE == 0xFEFF_FFFF);
    assert!(STAGE4_BASE == 0xFEBF_FFFF);
    assert!(STAGE5_BASE == 0xFE7F_FFFF);
};
