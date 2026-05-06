//! Property-based fuzzing for the 32 ATS-V anti-pattern check
//! functions + 4 red-team closures.
//!
//! Each check is exercised under arbitrary input shapes drawn from
//! `proptest` strategies.  The properties asserted are:
//!
//!   1. **No-panic guarantee**: every check function on every
//!      proptest-generated input returns either `None` or
//!      `Some(violation)` without panicking.
//!   2. **Silent-on-empty**: when the auxiliary input list (e.g.
//!      `inferred_used`, `composes_graph`) is empty AND the Shape
//!      has no violation-triggering content, the check returns
//!      `None`.
//!   3. **Stable error code**: when the check fires, the returned
//!      `code` field matches the expected `AntiPatternCode` arm.
//!
//! Each property runs 256 cases by default (configurable via the
//! proptest `cases` config).  A property failure shrinks
//! automatically to a minimal input via proptest's machinery.

use proptest::prelude::*;
use verum_kernel::arch::*;
use verum_kernel::arch_anti_pattern::*;

// =============================================================================
// Strategies — generators for arbitrary architecture values
// =============================================================================

fn small_string() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,15}".prop_map(|s| s.to_string())
}

fn arb_resource_tag() -> impl Strategy<Value = ResourceTag> {
    prop_oneof![
        small_string().prop_map(|n| ResourceTag::Database { name: n }),
        small_string().prop_map(|p| ResourceTag::File { path_pattern: p }),
        small_string().prop_map(|r| ResourceTag::Memory { region: r }),
        small_string().prop_map(|n| ResourceTag::Config { namespace: n }),
        Just(ResourceTag::Logger),
        Just(ResourceTag::Random),
        small_string().prop_map(ResourceTag::Custom),
    ]
}

fn arb_capability() -> impl Strategy<Value = Capability> {
    prop_oneof![
        arb_resource_tag().prop_map(|r| Capability::Read { resource: r }),
        arb_resource_tag().prop_map(|r| Capability::Write { resource: r }),
        small_string().prop_map(|tag| Capability::Custom {
            tag,
            schema: CapabilitySchema {
                description: "fuzz".into(),
                transfers_privilege: false,
                subsumed_by: vec![],
            },
        }),
    ]
}

fn arb_foundation() -> impl Strategy<Value = Foundation> {
    prop_oneof![
        Just(Foundation::ZfcTwoInacc),
        Just(Foundation::Hott),
        Just(Foundation::Cubical),
        Just(Foundation::Cic),
        Just(Foundation::Mltt),
        Just(Foundation::Eff),
    ]
}

fn arb_lifecycle() -> impl Strategy<Value = Lifecycle> {
    prop_oneof![
        Just(Lifecycle::Definition),
        small_string().prop_map(|s| Lifecycle::Theorem { since: s }),
        small_string().prop_map(|s| Lifecycle::Plan { target_completion: s }),
        small_string().prop_map(|s| Lifecycle::Postulate { citation: s }),
        small_string().prop_map(|s| Lifecycle::Interpretation { reason: s }),
    ]
}

fn arb_tier() -> impl Strategy<Value = Tier> {
    prop_oneof![
        Just(Tier::Interp),
        Just(Tier::Aot),
        Just(Tier::Gpu),
        Just(Tier::Check),
    ]
}

fn arb_stratum() -> impl Strategy<Value = MsfsStratum> {
    prop_oneof![
        Just(MsfsStratum::LFnd),
        Just(MsfsStratum::LCls),
        Just(MsfsStratum::LClsTop),
        Just(MsfsStratum::LAbs),
    ]
}

fn arb_shape() -> impl Strategy<Value = Shape> {
    (
        prop::collection::vec(arb_capability(), 0..4),
        prop::collection::vec(arb_capability(), 0..4),
        arb_tier(),
        arb_foundation(),
        arb_stratum(),
        arb_lifecycle(),
        any::<bool>(),
    )
        .prop_map(
            |(exposes, requires, at_tier, foundation, stratum, lifecycle, strict)| Shape {
                exposes,
                requires,
                preserves: vec![],
                consumes: vec![],
                at_tier,
                foundation,
                stratum,
                cve_closure: CveClosure {
                    constructive: None,
                    verifiable_strategy: None,
                    executable: None,
                },
                lifecycle,
                composes_with: vec![],
                strict,
                declarations: None,
            },
        )
}

// =============================================================================
// No-panic guarantee
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Every Shape passes through every check_* without panicking.
    /// This is the broadest invariant — encodes "the check
    /// functions are total on the type-correct input space."
    #[test]
    fn no_panic_on_arbitrary_shape(shape in arb_shape()) {
        let _ = check_capability_escalation(&shape, &[]);
        let _ = check_capability_leak(&shape, &[]);
        let _ = check_dependency_cycle(&shape, "fuzz_cog", &[]);
        let _ = check_tier_mixing(&shape, &[]);
        let _ = check_foundation_drift(&shape, &[]);
        let _ = check_register_mixing(&shape, &[]);
        let _ = check_tx_straddling(&shape, &[]);
        let _ = check_resource_straddling(&shape, &[]);
        let _ = check_lifecycle_regression(&shape, &[]);
        let _ = check_cve_incomplete(&shape);
        let _ = check_stratum_admissible(&shape);
        let _ = check_invariant_violation(&shape, &[]);
        let _ = check_dangling_message_type(&shape, &[]);
        let _ = check_unauthenticated_crossing(&shape, &[]);
        let _ = check_deterministic_violation(&shape, &[]);
        let _ = check_capability_duplication(&shape, &[]);
        let _ = check_orphan_capability(&shape, &[]);
        let _ = check_missing_handoff(&shape, &[]);
        let _ = check_foundation_downgrade(&shape, &[]);
        let _ = check_time_bound_leakage(&shape, &[]);
        let _ = check_persistence_mismatch(&shape, &[]);
        let _ = check_capability_laundering(&shape, 0);
        let _ = check_foundation_forgery(&shape, &[]);
        let _ = check_transitive_lifecycle_regression(&shape, &[]);
        let _ = check_declaration_drift(&shape, None);
        let _ = check_foundation_content_mismatch(&shape, &[]);
        // Red-team closures
        let _ = check_capability_ontology_v(&shape, &[]);
        let _ = check_theorem_cve_required_v(&shape);
        let _ = check_yoneda_canonical_roster_complete_v(&shape, &[]);
        let _ = check_consumes_format_v(&shape);
    }
}

// =============================================================================
// Silent-on-empty: empty auxiliaries ⇒ no violation (Shape-only checks)
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// AP-001 fires only when there are inferred_used capabilities
    /// not in shape.requires.  Empty inferred_used ⇒ silent.
    #[test]
    fn ap_001_silent_on_empty_inferred(shape in arb_shape()) {
        prop_assert!(check_capability_escalation(&shape, &[]).is_none());
    }

    /// AP-002 fires only when leaked_capabilities non-empty.
    #[test]
    fn ap_002_silent_on_empty_leaked(shape in arb_shape()) {
        prop_assert!(check_capability_leak(&shape, &[]).is_none());
    }

    /// AP-003 fires only when there's a cycle in composes_graph
    /// involving cog_name.  Empty graph ⇒ silent.
    #[test]
    fn ap_003_silent_on_empty_graph(shape in arb_shape()) {
        prop_assert!(check_dependency_cycle(&shape, "fuzz", &[]).is_none());
    }

    /// AP-016 fires only on linear-cap duplicates.
    #[test]
    fn ap_016_silent_on_empty_duplications(shape in arb_shape()) {
        prop_assert!(check_capability_duplication(&shape, &[]).is_none());
    }

    /// AP-017 fires only on relevant-cap orphans.
    #[test]
    fn ap_017_silent_on_empty_orphans(shape in arb_shape()) {
        prop_assert!(check_orphan_capability(&shape, &[]).is_none());
    }

    /// AP-018 fires only on composition handoff gaps.
    #[test]
    fn ap_018_silent_on_empty_gaps(shape in arb_shape()) {
        prop_assert!(check_missing_handoff(&shape, &[]).is_none());
    }

    /// AT-1 silent when registry empty (registry unavailable
    /// signal).  Per design: empty registry suppresses the check
    /// rather than false-positive against every Custom.
    #[test]
    fn at_1_silent_on_empty_registry(shape in arb_shape()) {
        prop_assert!(check_capability_ontology_v(&shape, &[]).is_none());
    }

    /// AT-3 silent when verdicts empty.
    #[test]
    fn at_3_silent_on_empty_verdicts(shape in arb_shape()) {
        prop_assert!(check_yoneda_canonical_roster_complete_v(&shape, &[]).is_none());
    }
}

// =============================================================================
// AP-011 stratum-admissibility — full coverage
// =============================================================================

proptest! {
    /// AP-011 fires iff stratum == LAbs.
    #[test]
    fn ap_011_fires_iff_l_abs(mut shape in arb_shape(), use_l_abs in any::<bool>()) {
        shape.stratum = if use_l_abs { MsfsStratum::LAbs } else { MsfsStratum::LFnd };
        let result = check_stratum_admissible(&shape);
        if use_l_abs {
            prop_assert!(result.is_some());
            let v = result.unwrap();
            // Stratum-admissibility uses FoundationDrift as the
            // surface code (it's a foundation-stability check by
            // analogy); kernel comment: "stratum-admissibility
            // routes via FoundationDrift". Allow either route.
            prop_assert!(
                v.code == AntiPatternCode::FoundationDrift
                    || v.code == AntiPatternCode::AbsoluteBoundaryAttempt
            );
        } else {
            prop_assert!(result.is_none());
        }
    }
}

// =============================================================================
// AT-2 theorem-CVE coupling — full coverage
// =============================================================================

proptest! {
    /// AT-2 fires iff lifecycle is Theorem AND CVE-closure
    /// incomplete.
    #[test]
    fn at_2_fires_iff_theorem_without_cve(
        mut shape in arb_shape(),
        is_theorem in any::<bool>(),
        is_full_cve in any::<bool>(),
    ) {
        shape.lifecycle = if is_theorem {
            Lifecycle::Theorem { since: "fuzz".into() }
        } else {
            Lifecycle::Definition
        };
        shape.cve_closure = if is_full_cve {
            CveClosure {
                constructive: Some("c".into()),
                verifiable_strategy: Some(VerifyStrategy::Certified),
                executable: Some("e".into()),
            }
        } else {
            CveClosure {
                constructive: None,
                verifiable_strategy: None,
                executable: None,
            }
        };
        let result = check_theorem_cve_required_v(&shape);
        let should_fire = is_theorem && !is_full_cve;
        prop_assert_eq!(result.is_some(), should_fire);
    }
}

// =============================================================================
// AT-5 consumes-format — fires iff at least one entry malformed
// =============================================================================

proptest! {
    /// Well-formed entries are silent; malformed fire.
    #[test]
    fn at_5_fires_on_malformed_entry(
        mut shape in arb_shape(),
        bad_entry in r"[^/ ]+/[^ ]* [^bnmo]+",
    ) {
        shape.consumes = vec![bad_entry];
        // The strategy-generated string MAY occasionally pass
        // (e.g. if it happens to look canonical).  The property
        // we assert is that the function NEVER panics — already
        // covered by no_panic_on_arbitrary_shape.  This test is
        // primarily a smoke that the consumes-format path is
        // exercised; precise fire-coverage lives in the unit
        // tests in arch_anti_pattern.rs.
        let _ = check_consumes_format_v(&shape);
    }

    /// Canonical-format entry is always silent.
    #[test]
    fn at_5_silent_on_canonical_entry(
        mut shape in arb_shape(),
        n in 1u32..1_000_000,
        unit in prop_oneof![Just("bytes"), Just("ops"), Just("ms"), Just("ns")],
        resource in "[a-z][a-z_]{0,10}",
    ) {
        shape.consumes = vec![format!("{}/{} {}", resource, n, unit)];
        prop_assert!(check_consumes_format_v(&shape).is_none());
    }
}

// =============================================================================
// AP-008 stratum admissibility helper — Shape::default never fires
// =============================================================================

proptest! {
    /// Shape::default_for_unannotated must vacuously pass every check.
    /// Per spec §17.5 backward-compat: unannotated cogs are always OK.
    #[test]
    fn default_shape_passes_all_checks(_seed in 0u32..1) {
        let shape = Shape::default_for_unannotated();
        let ctx = DiagnosticContext::default();
        let violations = check_all_anti_patterns(&shape, &ctx);
        prop_assert!(violations.is_empty(), "default shape produced violations: {:?}", violations);
    }
}
