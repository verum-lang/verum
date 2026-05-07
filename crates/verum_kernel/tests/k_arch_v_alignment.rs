//! Cross-side ATS-V alignment pin test.
//!
//! Holds the Verum-side `core/architecture/*.vr` declarations in
//! lockstep with the Rust-side `verum_kernel::arch*` enums.  Drift
//! in either direction fails this test with a concrete message
//! naming the missing or mismatched element.
//!
//! ## What this test pins
//!
//! 1. **Variant tag rosters** — every enum the kernel exposes
//!    through a `tag()` / `code()` / `name()` method has its
//!    canonical tag set hard-pinned here.  Adding or removing a
//!    variant on the kernel side requires updating this test.
//! 2. **Roster sizes** — every full-roster constant
//!    (`Observer::full_canonical_roster`, the 32-pattern
//!    AntiPatternCode list, the canonical-field roster) is pinned
//!    by size.
//! 3. **Verum-side variant presence** — the test reads every
//!    `core/architecture/*.vr` file as text and asserts that each
//!    canonical variant appears as a declaration token.
//! 4. **Verum-side helper presence** — the test asserts that each
//!    canonical helper name appears as a `pub fn` declaration on
//!    the Verum side.  This guarantees the operationalisation
//!    surface stays available to Verum cogs.
//!
//! ## Workflow when adding a variant
//!
//! 1. Update the kernel-side enum + impl method in
//!    `crates/verum_kernel/src/arch*.rs`.
//! 2. Update the Verum-side type + helper in
//!    `core/architecture/*.vr`.
//! 3. Update the canonical roster in this test.
//!
//! Skipping any step fails this test, blocking the change-set
//! from landing.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use verum_kernel::arch::*;
use verum_kernel::arch_anti_pattern::AntiPatternCode;
use verum_kernel::arch_mtac::*;

// =============================================================================
// Path resolution — find the workspace's core/ directory
// =============================================================================

fn workspace_core_architecture_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR points at crates/verum_kernel; walk up two
    // levels to reach the workspace root, then descend into core/.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolvable from crates/verum_kernel");
    workspace_root.join("core").join("architecture")
}

fn read_vr(name: &str) -> String {
    let path = workspace_core_architecture_dir().join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read {} for cross-side pin: {} (cwd: {:?})",
            path.display(),
            e,
            std::env::current_dir(),
        )
    })
}

fn assert_variant_in(text: &str, variant: &str, in_file: &str) {
    // The Verum-side .vr files declare variants either as
    //   | VariantName            (bare)
    //   | VariantName(...)       (tuple-style)
    //   | VariantName { ... }    (struct-style)
    // and reference them as `EnumName.VariantName`.  We accept any
    // of these surface forms.
    let surfaces = [
        format!("| {}", variant),
        format!("|{}", variant),
        format!(".{}", variant),
    ];
    let found = surfaces.iter().any(|s| text.contains(s));
    assert!(
        found,
        "Verum-side {}: missing variant `{}`. Expected one of declaration `| {}` or reference `.{}`.",
        in_file, variant, variant, variant,
    );
}

fn assert_helper_in(text: &str, name: &str, in_file: &str) {
    // Verum surface form: `public fn <name>` (the canonical visibility
    // marker is `public`, not Rust's `pub`).
    let needle = format!("public fn {}", name);
    assert!(
        text.contains(&needle),
        "Verum-side {}: missing `public fn {}` helper.  Operationalisation surface must mirror kernel-side `impl`.",
        in_file, name,
    );
}

// =============================================================================
// 1. Tier — 5 variants
// =============================================================================

#[test]
fn pin_tier_variants_aligned() {
    let kernel_tags: BTreeSet<&'static str> = [
        Tier::Interp.tag(),
        Tier::Aot.tag(),
        Tier::Gpu.tag(),
        Tier::Check.tag(),
        Tier::MultiTier { allowed: vec![] }.tag(),
    ]
    .iter()
    .copied()
    .collect();
    assert_eq!(kernel_tags.len(), 5, "Tier has 5 distinct tags");

    let expected = [
        "interp",
        "aot",
        "gpu",
        "check",
        "multi_tier",
    ]
    .iter()
    .copied()
    .collect::<BTreeSet<_>>();
    assert_eq!(
        kernel_tags, expected,
        "Tier kernel tags drifted from canonical roster"
    );

    let vr = read_vr("types.vr");
    for v in &["Interp", "Aot", "Gpu", "Check", "MultiTier"] {
        assert_variant_in(&vr, v, "core/architecture/types.vr (Tier)");
    }
    // Critical: TierCheck must NOT be present (was renamed to Check).
    assert!(
        !vr.contains("| TierCheck"),
        "Verum-side types.vr still declares `| TierCheck` — must be `| Check` for parser compatibility"
    );
}

// =============================================================================
// 2. MsfsStratum — 4 variants
// =============================================================================

#[test]
fn pin_msfs_stratum_variants_aligned() {
    let kernel_tags: BTreeSet<&'static str> = [
        MsfsStratum::LFnd.tag(),
        MsfsStratum::LCls.tag(),
        MsfsStratum::LClsTop.tag(),
        MsfsStratum::LAbs.tag(),
    ]
    .iter()
    .copied()
    .collect();
    let expected: BTreeSet<&'static str> =
        ["l_fnd", "l_cls", "l_cls_top", "l_abs"].iter().copied().collect();
    assert_eq!(kernel_tags, expected, "MsfsStratum kernel tags drifted");

    let vr = read_vr("types.vr");
    for v in &["LFnd", "LCls", "LClsTop", "LAbs"] {
        assert_variant_in(&vr, v, "core/architecture/types.vr (MsfsStratum)");
    }
    assert_helper_in(&vr, "stratum_is_admissible", "types.vr");
    assert_helper_in(&vr, "stratum_tag", "types.vr");
}

// =============================================================================
// 3. Foundation — 7 variants
// =============================================================================

#[test]
fn pin_foundation_variants_aligned() {
    let kernel_tags: BTreeSet<&'static str> = [
        Foundation::ZfcTwoInacc.tag(),
        Foundation::Hott.tag(),
        Foundation::Cubical.tag(),
        Foundation::Cic.tag(),
        Foundation::Mltt.tag(),
        Foundation::Eff.tag(),
        Foundation::Custom {
            name: "x".into(),
            framework_corpus: "y".into(),
        }
        .tag(),
    ]
    .iter()
    .copied()
    .collect();
    let expected: BTreeSet<&'static str> = [
        "zfc_two_inacc", "hott", "cubical", "cic", "mltt", "eff", "custom",
    ]
    .iter()
    .copied()
    .collect();
    assert_eq!(kernel_tags, expected, "Foundation kernel tags drifted");

    let vr = read_vr("types.vr");
    for v in &[
        "ZfcTwoInacc",
        "Hott",
        "Cubical",
        "Cic",
        "Mltt",
        "Eff",
    ] {
        assert_variant_in(&vr, v, "core/architecture/types.vr (Foundation)");
    }
    assert_helper_in(&vr, "foundation_directly_subsumed_by", "types.vr");
}

// =============================================================================
// 4. Lifecycle — 9 variants
// =============================================================================

#[test]
fn pin_lifecycle_variants_aligned() {
    let kernel_tags: BTreeSet<&'static str> = [
        Lifecycle::Hypothesis {
            confidence: ConfidenceLevel::Medium,
        }
        .tag(),
        Lifecycle::Plan {
            target_completion: "x".into(),
        }
        .tag(),
        Lifecycle::Postulate {
            citation: "x".into(),
        }
        .tag(),
        Lifecycle::Definition.tag(),
        Lifecycle::Conditional { conditions: vec![] }.tag(),
        Lifecycle::Theorem {
            since: "v0.1".into(),
        }
        .tag(),
        Lifecycle::Interpretation {
            reason: "x".into(),
        }
        .tag(),
        Lifecycle::Retracted {
            reason: "x".into(),
            replacement: None,
        }
        .tag(),
        Lifecycle::Obsolete {
            deprecation_reason: "x".into(),
            replacement: None,
        }
        .tag(),
    ]
    .iter()
    .copied()
    .collect();
    assert_eq!(kernel_tags.len(), 9, "Lifecycle has 9 distinct tags");

    let vr = read_vr("types.vr");
    for v in &[
        "Hypothesis",
        "Plan",
        "Postulate",
        "Definition",
        "Conditional",
        "Theorem",
        "Interpretation",
        "Retracted",
        "Obsolete",
    ] {
        assert_variant_in(&vr, v, "core/architecture/types.vr (Lifecycle)");
    }
    assert_helper_in(&vr, "lifecycle_rank", "types.vr");
    assert_helper_in(&vr, "lifecycle_cve_glyph", "types.vr");
    assert_helper_in(&vr, "lifecycle_is_mature_corpus_forbidden", "types.vr");
}

// =============================================================================
// 5. Capability — 9 variants
// =============================================================================

#[test]
fn pin_capability_variants_aligned() {
    let kernel_tags: BTreeSet<&'static str> = [
        Capability::Read {
            resource: ResourceTag::Logger,
        }
        .tag(),
        Capability::Write {
            resource: ResourceTag::Logger,
        }
        .tag(),
        Capability::Exec {
            target: ExecTarget::Custom("x".into()),
        }
        .tag(),
        Capability::Escalate {
            realm: PrivilegeRealm::Admin,
        }
        .tag(),
        Capability::Spawn {
            lifetime: TaskLifetime::Detached,
        }
        .tag(),
        Capability::TimeBound {
            until: ExpirationPolicy::AfterDuration { milliseconds: 1 },
        }
        .tag(),
        Capability::Persist {
            medium: PersistenceMedium::Disk { path: "/x".into() },
        }
        .tag(),
        Capability::Network {
            protocol: NetProtocol::Tcp,
            direction: NetDirection::Inbound,
        }
        .tag(),
        Capability::Custom {
            tag: "x".into(),
            schema: CapabilitySchema {
                description: "x".into(),
                transfers_privilege: false,
                subsumed_by: vec![],
            },
        }
        .tag(),
    ]
    .iter()
    .copied()
    .collect();
    assert_eq!(kernel_tags.len(), 9, "Capability has 9 distinct tags");

    let vr = read_vr("types.vr");
    for v in &[
        "Read",
        "Write",
        "Exec",
        "Escalate",
        "Spawn",
        "TimeBound",
        "Persist",
        "Network",
        "CustomCapability",
    ] {
        assert_variant_in(&vr, v, "core/architecture/types.vr (Capability)");
    }
    assert_helper_in(&vr, "capability_tag", "types.vr");
    // CapabilitySchema mirror present.
    assert!(
        vr.contains("CapabilitySchema"),
        "Verum-side types.vr missing CapabilitySchema type"
    );
}

// =============================================================================
// 6. VerifyStrategy — 9 levels strictly ordered
// =============================================================================

#[test]
fn pin_verify_strategy_aligned() {
    let order = [
        VerifyStrategy::Runtime,
        VerifyStrategy::Static,
        VerifyStrategy::Fast,
        VerifyStrategy::Formal,
        VerifyStrategy::Proof,
        VerifyStrategy::Thorough,
        VerifyStrategy::Reliable,
        VerifyStrategy::Certified,
        VerifyStrategy::Synthesize,
    ];
    for window in order.windows(2) {
        assert!(
            window[0].rank() < window[1].rank(),
            "VerifyStrategy rank not strictly increasing: {:?} >= {:?}",
            window[0],
            window[1]
        );
    }

    let vr = read_vr("types.vr");
    for v in &[
        "Runtime",
        "Static",
        "Fast",
        "Formal",
        "Proof",
        "Thorough",
        "Reliable",
        "Certified",
        "Synthesize",
    ] {
        assert_variant_in(&vr, v, "core/architecture/types.vr (VerifyStrategy)");
    }
    assert_helper_in(&vr, "verify_strategy_rank", "types.vr");
}

// =============================================================================
// 7. AntiPatternCode — 40 canonical codes (32 base + 8 CVE-AH band)
// =============================================================================

#[test]
fn pin_anti_pattern_code_count_canonical() {
    let all = AntiPatternCode::full_list();
    assert_eq!(
        all.len(),
        40,
        "Kernel-side AntiPatternCode roster size drifted from canonical 40"
    );

    let codes: BTreeSet<&'static str> = all.iter().map(|c| c.code()).collect();
    assert_eq!(codes.len(), 40, "AntiPatternCode codes not unique");

    // Verify the AP-001..AP-040 pattern is fully covered.
    for n in 1..=40 {
        let expected = format!("ATS-V-AP-{:03}", n);
        assert!(
            codes.contains(expected.as_str()),
            "Missing AntiPatternCode {}",
            expected
        );
    }

    let vr = read_vr("anti_patterns.vr");
    let names = [
        // Capability / composition core (AP-001..010)
        "CapabilityEscalation",
        "CapabilityLeak",
        "DependencyCycle",
        "TierMixing",
        "FoundationDrift",
        "RegisterMixing",
        "TxStraddling",
        "ResourceStraddling",
        "LifecycleRegression",
        "CveIncomplete",
        // Boundary / lifecycle / capability ontology (AP-011..026)
        "AbsoluteBoundaryAttempt",
        "InvariantViolation",
        "DanglingMessageType",
        "UnauthenticatedCrossing",
        "DeterministicViolation",
        "CapabilityDuplication",
        "OrphanCapability",
        "MissingHandoff",
        "FoundationDowngrade",
        "TimeBoundLeakage",
        "PersistenceMismatch",
        "CapabilityLaundering",
        "FoundationForgery",
        "TransitiveLifecycleRegression",
        "DeclarationDrift",
        "FoundationContentMismatch",
        // MTAC modal-temporal (AP-027..032)
        "TemporalInconsistency",
        "CounterfactualBrittleness",
        "MissedAdjoint",
        "UniversalPropertyViolation",
        "PhantomEvolution",
        "YonedaInequivalentRefactor",
        // CVE articulation-hygiene band (AP-033..040) —
        // operationalises cve-architecture spec §1.5, §2.3.0, §3.5,
        // §4.5, §14.6, §16. AP-040 closes architectural-revision
        // open invariant R4 (self-reference without operator+Fix).
        "RetractedCitationUse",
        "HypothesisWithoutMaturationPlan",
        "InterpretationInMatureCorpus",
        "ObserverImpersonation",
        "BoundlessAudit",
        "ImplicitSubstrate",
        "AnchoringOverextension",
        "SelfReferenceWithoutOperator",
    ];
    for n in &names {
        assert_variant_in(&vr, n, "core/architecture/anti_patterns.vr (AntiPatternCode)");
    }
    assert_helper_in(&vr, "anti_pattern_code_str", "anti_patterns.vr");
    assert_helper_in(&vr, "anti_pattern_full_roster", "anti_patterns.vr");
}

// =============================================================================
// 7b. CVE-architecture spec primitives — Verum/kernel alignment
// =============================================================================

#[test]
fn pin_executability_sense_three_canonical() {
    use verum_kernel::arch::ExecutabilitySense;
    let canonical = [
        ExecutabilitySense::StructuralReadiness,
        ExecutabilitySense::CurrentExecution,
        ExecutabilitySense::PostFactumChronicle,
    ];
    let tags: BTreeSet<&'static str> = canonical.iter().map(|s| s.tag()).collect();
    assert_eq!(tags.len(), 3, "ExecutabilitySense tags not unique");
    // Soundness pin (cve-architecture spec §2.3.0): exactly one
    // sense (StructuralReadiness) is the canonical content of CVE-E.
    assert!(ExecutabilitySense::StructuralReadiness.is_canonical_e());
    assert!(!ExecutabilitySense::CurrentExecution.is_canonical_e());
    assert!(!ExecutabilitySense::PostFactumChronicle.is_canonical_e());

    let vr = read_vr("types.vr");
    for v in &[
        "StructuralReadiness",
        "CurrentExecution",
        "PostFactumChronicle",
    ] {
        assert_variant_in(&vr, v, "core/architecture/types.vr (ExecutabilitySense)");
    }
    assert_helper_in(&vr, "executability_sense_tag", "types.vr");
    assert_helper_in(&vr, "executability_sense_is_canonical_e", "types.vr");
    assert_helper_in(&vr, "executability_sense_canonical_unique", "types.vr");
}

#[test]
fn pin_cognitive_substrate_four_canonical() {
    use verum_kernel::arch::CognitiveSubstrate;
    let canonical = [
        CognitiveSubstrate::AnalyticDecompositional,
        CognitiveSubstrate::HolisticRelational,
        CognitiveSubstrate::ActionCentric,
        CognitiveSubstrate::TraditionTransmitting,
    ];
    let tags: BTreeSet<&'static str> = canonical.iter().map(|s| s.tag()).collect();
    assert_eq!(tags.len(), 4, "CognitiveSubstrate tags not unique");
    // Soundness pin (cve-architecture spec §1.5): the default for
    // ATS-V is AnalyticDecompositional.
    assert_eq!(
        CognitiveSubstrate::default_for_ats_v(),
        CognitiveSubstrate::AnalyticDecompositional
    );

    let vr = read_vr("types.vr");
    for v in &[
        "AnalyticDecompositional",
        "HolisticRelational",
        "ActionCentric",
        "TraditionTransmitting",
    ] {
        assert_variant_in(&vr, v, "core/architecture/types.vr (CognitiveSubstrate)");
    }
    assert_helper_in(&vr, "cognitive_substrate_tag", "types.vr");
    assert_helper_in(&vr, "cognitive_substrate_default", "types.vr");
}

#[test]
fn pin_formal_anchoring_seven_variants() {
    use verum_kernel::arch::FormalAnchoring;
    let canonical = [
        FormalAnchoring::CurryHowardLawvere,
        FormalAnchoring::AutomataTheory,
        FormalAnchoring::ControlTheory,
        FormalAnchoring::DistributedProtocols,
        FormalAnchoring::FunctionalSystems,
        FormalAnchoring::InstitutionalDesign,
        FormalAnchoring::CustomAnchoring("custom".to_string()),
    ];
    let tags: BTreeSet<&'static str> = canonical.iter().map(|s| s.tag()).collect();
    assert_eq!(
        tags.len(),
        7,
        "FormalAnchoring tags must be 7 distinct (CHL + 5 parallel anchorings + Custom)"
    );
    // Soundness pin (cve-architecture spec §4.5): the default
    // anchoring is the CHL eponym.
    assert_eq!(
        FormalAnchoring::default_for_ats_v(),
        FormalAnchoring::CurryHowardLawvere
    );

    let vr = read_vr("types.vr");
    for v in &[
        "CurryHowardLawvere",
        "AutomataTheory",
        "ControlTheory",
        "DistributedProtocols",
        "FunctionalSystems",
        "InstitutionalDesign",
        "CustomAnchoring",
    ] {
        assert_variant_in(&vr, v, "core/architecture/types.vr (FormalAnchoring)");
    }
    assert_helper_in(&vr, "formal_anchoring_tag", "types.vr");
    assert_helper_in(&vr, "formal_anchoring_default", "types.vr");
}

#[test]
fn pin_purpose_threshold_axes() {
    use verum_kernel::arch::{
        CveThresholdE, CveThresholdK, CveThresholdV, Purpose,
    };
    // K threshold has 3 modes, V has 3, E has 3 — 3³ = 27 cells of
    // declared-purpose space (cve-architecture spec §14.6).
    assert_eq!(CveThresholdK::FullWitness.tag(), "full_witness");
    assert_eq!(CveThresholdK::TypedSchema.tag(), "typed_schema");
    assert_eq!(
        CveThresholdK::ReferenceImplBounded.tag(),
        "reference_impl_bounded"
    );
    assert_eq!(CveThresholdV::FullFormalProof.tag(), "full_formal_proof");
    assert_eq!(
        CveThresholdV::TypecheckPlusTests.tag(),
        "typecheck_plus_tests"
    );
    assert_eq!(
        CveThresholdV::NamedCertification.tag(),
        "named_certification"
    );
    assert_eq!(CveThresholdE::StructurallyReady.tag(), "structurally_ready");
    assert_eq!(CveThresholdE::DeployedInOneEnv.tag(), "deployed_in_one_env");
    assert_eq!(CveThresholdE::FunctorialOnly.tag(), "functorial_only");

    let p = Purpose::default_unspecified();
    assert_eq!(p.role, "unspecified");
    assert_eq!(p.k_min, CveThresholdK::FullWitness);
    assert_eq!(p.v_min, CveThresholdV::TypecheckPlusTests);
    assert_eq!(p.e_min, CveThresholdE::StructurallyReady);

    let vr = read_vr("types.vr");
    for v in &[
        "FullWitness",
        "TypedSchema",
        "ReferenceImplBounded",
        "FullFormalProof",
        "TypecheckPlusTests",
        "NamedCertification",
        "StructurallyReady",
        "DeployedInOneEnv",
        "FunctorialOnly",
    ] {
        assert_variant_in(&vr, v, "core/architecture/types.vr (CveThreshold*)");
    }
    assert_helper_in(&vr, "purpose_default", "types.vr");
}

#[test]
fn pin_shape_declarations_extension() {
    use verum_kernel::arch::ShapeDeclarations;
    let empty = ShapeDeclarations::empty();
    assert!(empty.purpose.is_none());
    assert!(empty.substrate.is_none());
    assert!(empty.anchoring.is_none());
    assert!(empty.e_sense.is_none());
    assert!(empty.self_reference.is_none());
    let vr = read_vr("types.vr");
    assert_helper_in(&vr, "shape_declarations_empty", "types.vr");
}

#[test]
fn pin_fixpoint_class_four_canonical() {
    use verum_kernel::arch::{
        EndomorphismClass, FixpointCategory, FixpointClass, FixpointTheorem,
    };

    // Smart constructors produce the canonical (category, endomorphism,
    // theorem) triples per the universal-property classifier
    // (articulation-hygiene §8.1.fixpoint-class-universal).
    let banach = FixpointClass::banach();
    assert_eq!(banach.category, FixpointCategory::CompleteMetricSpace);
    assert_eq!(banach.endomorphism_class, EndomorphismClass::Contracting);
    assert_eq!(banach.theorem, FixpointTheorem::Banach);
    assert!(banach.is_canonical());

    let tarski = FixpointClass::tarski();
    assert_eq!(tarski.category, FixpointCategory::CompleteLattice);
    assert_eq!(tarski.endomorphism_class, EndomorphismClass::Monotone);
    assert_eq!(tarski.theorem, FixpointTheorem::Tarski);
    assert!(tarski.is_canonical());

    let adamek = FixpointClass::adamek();
    assert_eq!(adamek.category, FixpointCategory::CocompleteCategory);
    assert_eq!(
        adamek.endomorphism_class,
        EndomorphismClass::ContinuousFunctor
    );
    assert_eq!(adamek.theorem, FixpointTheorem::Adamek);
    assert!(adamek.is_canonical());

    let custom = FixpointClass::custom_fixpoint("any");
    assert!(matches!(
        custom.theorem,
        FixpointTheorem::Custom(ref s) if s == "any"
    ));
    assert!(!custom.is_canonical());

    // The four classes have pairwise distinct tags.
    let canonical = [&banach, &tarski, &adamek, &custom];
    let tags: BTreeSet<&'static str> = canonical.iter().map(|f| f.tag()).collect();
    assert_eq!(
        tags.len(),
        4,
        "FixpointClass tags must be 4 distinct (Banach + Tarski + Adamek + Custom)"
    );

    // Cross-side parity: Verum-side helpers exist with matching names.
    let vr = read_vr("types.vr");
    for v in &[
        "Banach",
        "Tarski",
        "Adamek",
        "Custom",
        "CompleteMetricSpace",
        "CompleteLattice",
        "CocompleteCategory",
        "CustomCategory",
        "Contracting",
        "Monotone",
        "ContinuousFunctor",
        "CustomEndomorphismClass",
    ] {
        assert_variant_in(&vr, v, "core/architecture/types.vr (FixpointClass universal-property)");
    }
    assert_helper_in(&vr, "fixpoint_class_tag", "types.vr");
    assert_helper_in(&vr, "fixpoint_class_is_canonical", "types.vr");
    assert_helper_in(&vr, "fixpoint_class_banach", "types.vr");
    assert_helper_in(&vr, "fixpoint_class_tarski", "types.vr");
    assert_helper_in(&vr, "fixpoint_class_adamek", "types.vr");
    assert_helper_in(&vr, "fixpoint_class_custom_fixpoint", "types.vr");
    assert_helper_in(&vr, "fixpoint_class_tags_distinct", "types.vr");
    assert_helper_in(&vr, "fixpoint_class_canonical_triples", "types.vr");
}

#[test]
fn pin_seven_configurations_closure_exhaustive() {
    // Pin: CVE seven-cell closure (seven-configurations §9 of the
    // website). Every cell of CveAxisMode³ must map to a productive
    // glyph; cross-side parity is enforced.
    use verum_kernel::arch::{
        CveAxisMode, is_productive_glyph,
        seven_configurations_closure_exhaustive,
        seven_configurations_closure_witness,
    };

    // Kernel-side exhaustive predicate.
    assert!(
        seven_configurations_closure_exhaustive(),
        "seven_configurations_closure_witness must produce a productive \
         glyph for every (c, v, e) ∈ CveAxisMode³"
    );

    // Spot-check the named cells of seven-configurations §1
    // (the seven productive configurations) against the witness.
    use CveAxisMode::*;
    assert_eq!(
        seven_configurations_closure_witness(Positive, Positive, Positive),
        "[T]",
        "cell 1 of §1 — Theorem"
    );
    assert_eq!(
        seven_configurations_closure_witness(Positive, Absent, Absent),
        "[H]",
        "cell 9 of §9 — Hypothesis"
    );
    assert_eq!(
        seven_configurations_closure_witness(Absent, Absent, Absent),
        "[I]",
        "cell 27 of §9 — Interpretation"
    );

    // Productive alphabet predicate.
    assert!(is_productive_glyph("[T]"));
    assert!(is_productive_glyph("[D]"));
    assert!(is_productive_glyph("[C]"));
    assert!(is_productive_glyph("[P]"));
    assert!(is_productive_glyph("[H]"));
    assert!(is_productive_glyph("[I]"));
    assert!(
        !is_productive_glyph("[✗]"),
        "[✗] is a meta-state, not a productive glyph"
    );

    // Tag tags are pairwise distinct.
    assert_ne!(CveAxisMode::Positive.tag(), CveAxisMode::Partial.tag());
    assert_ne!(CveAxisMode::Partial.tag(), CveAxisMode::Absent.tag());
    assert_ne!(CveAxisMode::Positive.tag(), CveAxisMode::Absent.tag());

    // Cross-side parity: Verum-side helpers exist with matching names.
    let vr = read_vr("types.vr");
    assert_helper_in(&vr, "cve_axis_mode_tag", "types.vr");
    assert_helper_in(&vr, "seven_configurations_closure_witness", "types.vr");
    assert_helper_in(&vr, "seven_configurations_closure_exhaustive", "types.vr");
}

#[test]
fn pin_self_reference_witness_format() {
    use verum_kernel::arch::{FixpointClass, FixpointTheorem, SelfReferenceWitness};
    let w = SelfReferenceWitness::unspecified();
    assert_eq!(w.operator, "unspecified");
    assert_eq!(w.fixed_point, "unspecified");
    assert!(matches!(
        w.fixpoint_class.theorem,
        FixpointTheorem::Custom(_)
    ));
    assert!(!w.fixpoint_class.is_canonical());

    // Concrete witness construction roundtrip.
    let w2 = SelfReferenceWitness {
        operator: "synarc.governance.amendment_operator".to_string(),
        fixed_point: "synarc.governance.constitution_v1".to_string(),
        fixpoint_class: FixpointClass::banach(),
    };
    assert_eq!(w2.fixpoint_class.tag(), "banach");
    assert!(w2.fixpoint_class.is_canonical());

    let vr = read_vr("types.vr");
    assert_helper_in(&vr, "self_reference_witness_unspecified", "types.vr");
}

#[test]
fn pin_architectural_defect_format() {
    use verum_kernel::arch::{
        ArchitecturalDefect, DefectKind, Resolution,
    };
    // Pin: cve-architecture spec §20.4 record format — name, version,
    // submitted_on, submitter, kind, witness, context, observed,
    // expected, resolution.
    let d = ArchitecturalDefect {
        short_name: "test".into(),
        arch_version: "v0.1".into(),
        submitted_on: "2026-05-06".into(),
        submitter: "auditor".into(),
        kind: DefectKind::FalseRejection,
        witness_artefact: "cog::x".into(),
        application_context: "ATS-V phase".into(),
        observed_result: "rejected".into(),
        expected_result: "accepted".into(),
        proposed_resolution: Resolution::L2Refinement,
    };
    assert_eq!(d.kind.tag(), "false_rejection");
    assert_eq!(d.proposed_resolution.tag(), "l2_refinement");

    let vr = read_vr("types.vr");
    for v in &[
        "FalseRejection",
        "FalseAcceptance",
        "InterLayerLeak",
        "OtherDefect",
        "L4Revision",
        "L2Refinement",
        "OtherResolution",
    ] {
        assert_variant_in(&vr, v, "core/architecture/types.vr (DefectKind/Resolution)");
    }
}

// =============================================================================
// 8. Observer — canonical 5-roster
// =============================================================================

#[test]
fn pin_observer_canonical_roster_size_five() {
    let roster = Observer::full_canonical_roster();
    assert_eq!(
        roster.len(),
        5,
        "Observer canonical roster size drifted from 5 (the Yoneda invariant)"
    );

    let tags: BTreeSet<&'static str> = roster.iter().map(|o| o.tag()).collect();
    let expected: BTreeSet<&'static str> = [
        "end_user",
        "peer_cog",
        "stakeholder",
        "auditor",
        "adversary",
    ]
    .iter()
    .copied()
    .collect();
    assert_eq!(tags, expected, "Observer roster tags drifted");

    let vr = read_vr("mtac.vr");
    for v in &["EndUser", "PeerCog", "Stakeholder", "Auditor", "Adversary"] {
        assert_variant_in(&vr, v, "core/architecture/mtac.vr (Observer)");
    }
    assert_helper_in(&vr, "observer_full_canonical_roster", "mtac.vr");
    assert_helper_in(&vr, "observer_roster_size_invariant", "mtac.vr");
}

// =============================================================================
// 9. ModalAssertion — 6 operators with disjoint modal/temporal sets
// =============================================================================

#[test]
fn pin_modal_assertion_six_operators() {
    let probes = [
        ModalAssertion::Necessity {
            proposition: ArchProposition::FoundationStable,
        },
        ModalAssertion::Possibility {
            proposition: ArchProposition::FoundationStable,
        },
        ModalAssertion::Eventually {
            proposition: ArchProposition::FoundationStable,
        },
        ModalAssertion::Always {
            proposition: ArchProposition::FoundationStable,
        },
        ModalAssertion::Until {
            first: ArchProposition::FoundationStable,
            second: ArchProposition::FoundationStable,
        },
        ModalAssertion::Counterfactual {
            antecedent: ArchProposition::FoundationStable,
            consequent: ArchProposition::FoundationStable,
        },
    ];
    let tags: BTreeSet<&'static str> = probes.iter().map(|p| p.tag()).collect();
    assert_eq!(tags.len(), 6, "ModalAssertion 6 operators distinct");

    // Modal and temporal arms are disjoint.
    let n = &probes[0];
    let e = &probes[2];
    assert!(n.is_modal() && !n.is_temporal());
    assert!(e.is_temporal() && !e.is_modal());

    let vr = read_vr("mtac.vr");
    for v in &[
        "Necessity",
        "Possibility",
        "Eventually",
        "Always",
        "Until",
        "CounterfactualImpl",
    ] {
        assert_variant_in(&vr, v, "core/architecture/mtac.vr (ModalAssertion)");
    }
    assert_helper_in(&vr, "modal_is_temporal", "mtac.vr");
    assert_helper_in(&vr, "modal_is_modal", "mtac.vr");
}

// =============================================================================
// 10. CveClosure — 0..=3 degree
// =============================================================================

#[test]
fn pin_cve_closure_degree_count_arms() {
    let full = CveClosure {
        constructive: Some("c".into()),
        verifiable_strategy: Some(VerifyStrategy::Certified),
        executable: Some("e".into()),
    };
    assert_eq!(full.closure_degree(), 3);
    assert!(full.is_fully_closed());

    let none = CveClosure {
        constructive: None,
        verifiable_strategy: None,
        executable: None,
    };
    assert_eq!(none.closure_degree(), 0);

    let vr = read_vr("types.vr");
    assert_helper_in(&vr, "cve_closure_degree", "types.vr");
    assert_helper_in(&vr, "cve_closure_is_fully_closed", "types.vr");
}

// =============================================================================
// 11. Composition / corpus / phase / parse modules — presence
// =============================================================================

#[test]
fn pin_composition_module_present() {
    let vr = read_vr("composition.vr");
    assert!(
        vr.contains("CompositionResult"),
        "core/architecture/composition.vr missing CompositionResult type"
    );
    assert!(
        vr.contains("kernel_arch_composition_engine"),
        "core/architecture/composition.vr missing kernel_arch_composition_engine axiom"
    );
    assert!(
        vr.contains("kernel_arch_composition_associative"),
        "core/architecture/composition.vr missing kernel_arch_composition_associative axiom"
    );
    assert_helper_in(&vr, "composition_result_is_composed", "composition.vr");
}

#[test]
fn pin_corpus_module_present() {
    let vr = read_vr("corpus.vr");
    for v in &[
        "NoCircularDependencies",
        "FoundationConsistency",
        "NoLAbsClaim",
        "CapabilityClosure",
    ] {
        assert_variant_in(&vr, v, "core/architecture/corpus.vr (CorpusInvariant)");
    }
    assert_helper_in(&vr, "corpus_invariant_full_list", "corpus.vr");
    assert_helper_in(&vr, "corpus_invariant_roster_size_invariant", "corpus.vr");
}

#[test]
fn pin_phase_module_present() {
    let vr = read_vr("phase.vr");
    assert!(
        vr.contains("ArchPhaseReport"),
        "core/architecture/phase.vr missing ArchPhaseReport type"
    );
    assert!(
        vr.contains("ModuleArchResult"),
        "core/architecture/phase.vr missing ModuleArchResult type"
    );
    assert_helper_in(&vr, "arch_phase_report_is_load_bearing", "phase.vr");
    assert_helper_in(&vr, "module_arch_result_is_load_bearing", "phase.vr");
}

#[test]
fn pin_parse_module_present() {
    let vr = read_vr("parse.vr");
    for v in &[
        "UnknownField",
        "InvalidValue",
        "MissingRequired",
        "UnknownVariant",
        "NotAnArchModuleAttribute",
    ] {
        assert_variant_in(&vr, v, "core/architecture/parse.vr (ArchParseError)");
    }
    assert_helper_in(&vr, "arch_module_canonical_fields", "parse.vr");
    assert_helper_in(&vr, "arch_module_field_count_invariant", "parse.vr");
}

// =============================================================================
// 12. Red-team closure axioms — must exist on Verum side
// =============================================================================

#[test]
fn pin_red_team_closure_axioms_present() {
    let vr = read_vr("anti_patterns.vr");
    for axiom in &[
        "kernel_arch_capability_ontology_check",
        "kernel_arch_yoneda_canonical_roster_complete",
        "kernel_arch_theorem_cve_required",
        "kernel_arch_consumes_format_check",
    ] {
        assert!(
            vr.contains(axiom),
            "core/architecture/anti_patterns.vr missing red-team closure axiom `{}` — \
             attack vectors AT-1/AT-2/AT-3/AT-5 require all four declarations",
            axiom,
        );
    }
}

// =============================================================================
// 13. mod.vr — re-exports the full surface
// =============================================================================

#[test]
fn pin_mod_re_exports_full_surface() {
    let vr = read_vr("mod.vr");
    let expected_modules = [
        "super.types",
        "super.anti_patterns",
        "super.composition",
        "super.corpus",
        "super.phase",
        "super.parse",
        "super.mtac",
        "super.counterfactual",
        "super.adjunction",
        "super.yoneda",
    ];
    for m in &expected_modules {
        assert!(
            vr.contains(m),
            "core/architecture/mod.vr does not re-export {} — full ATS-V surface must be visible from `core.architecture.mod`",
            m,
        );
    }
}

// =============================================================================
// 14. Anti-pattern check-function coverage — all 32 codes must have an impl
// =============================================================================

#[test]
fn pin_all_canonical_codes_have_check_function() {
    // For every AntiPatternCode, a `check_*` function must exist on
    // the kernel side that fires the violation under at least one
    // input.  The pin test does not invoke the dispatcher — it
    // enumerates the canonical roster against a hand-pinned list of
    // `check_*` function names that the source must contain.
    let kernel_arch_anti_pattern_src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("arch_anti_pattern.rs"),
    )
    .expect("kernel source readable");

    // Mapping: AntiPatternCode → expected `pub fn check_*` name.
    // When two codes share a common dispatcher (e.g. AT-1 surfaces
    // under CapabilityEscalation), the corresponding check function
    // is deliberately listed twice.
    let expected_fns: &[(AntiPatternCode, &str)] = &[
        (AntiPatternCode::CapabilityEscalation, "check_capability_escalation"),
        (AntiPatternCode::CapabilityLeak, "check_capability_leak"),
        (AntiPatternCode::DependencyCycle, "check_dependency_cycle"),
        (AntiPatternCode::TierMixing, "check_tier_mixing"),
        (AntiPatternCode::FoundationDrift, "check_foundation_drift"),
        (AntiPatternCode::RegisterMixing, "check_register_mixing"),
        (AntiPatternCode::TxStraddling, "check_tx_straddling"),
        (AntiPatternCode::ResourceStraddling, "check_resource_straddling"),
        (AntiPatternCode::LifecycleRegression, "check_lifecycle_regression"),
        (AntiPatternCode::CveIncomplete, "check_cve_incomplete"),
        (AntiPatternCode::AbsoluteBoundaryAttempt, "check_stratum_admissible"),
        (AntiPatternCode::InvariantViolation, "check_invariant_violation"),
        (AntiPatternCode::DanglingMessageType, "check_dangling_message_type"),
        (AntiPatternCode::UnauthenticatedCrossing, "check_unauthenticated_crossing"),
        (AntiPatternCode::DeterministicViolation, "check_deterministic_violation"),
        (AntiPatternCode::CapabilityDuplication, "check_capability_duplication"),
        (AntiPatternCode::OrphanCapability, "check_orphan_capability"),
        (AntiPatternCode::MissingHandoff, "check_missing_handoff"),
        (AntiPatternCode::FoundationDowngrade, "check_foundation_downgrade"),
        (AntiPatternCode::TimeBoundLeakage, "check_time_bound_leakage"),
        (AntiPatternCode::PersistenceMismatch, "check_persistence_mismatch"),
        (AntiPatternCode::CapabilityLaundering, "check_capability_laundering"),
        (AntiPatternCode::FoundationForgery, "check_foundation_forgery"),
        (
            AntiPatternCode::TransitiveLifecycleRegression,
            "check_transitive_lifecycle_regression",
        ),
        (AntiPatternCode::DeclarationDrift, "check_declaration_drift"),
        (
            AntiPatternCode::FoundationContentMismatch,
            "check_foundation_content_mismatch",
        ),
        (AntiPatternCode::TemporalInconsistency, "check_temporal_inconsistency"),
        (
            AntiPatternCode::CounterfactualBrittleness,
            "check_counterfactual_brittleness",
        ),
        (AntiPatternCode::MissedAdjoint, "check_missed_adjoint"),
        (
            AntiPatternCode::UniversalPropertyViolation,
            "check_universal_property_violation",
        ),
        (AntiPatternCode::PhantomEvolution, "check_phantom_evolution"),
        (
            AntiPatternCode::YonedaInequivalentRefactor,
            "check_yoneda_inequivalent_refactor",
        ),
        // CVE articulation-hygiene band (AP-033..039) — operationalises
        // cve-architecture spec §1.5, §2.3.0, §3.5, §4.5, §14.6, §16.
        (
            AntiPatternCode::RetractedCitationUse,
            "check_retracted_citation_use",
        ),
        (
            AntiPatternCode::HypothesisWithoutMaturationPlan,
            "check_hypothesis_without_maturation_plan",
        ),
        (
            AntiPatternCode::InterpretationInMatureCorpus,
            "check_interpretation_in_mature_corpus",
        ),
        (
            AntiPatternCode::ObserverImpersonation,
            "check_observer_impersonation",
        ),
        (
            AntiPatternCode::BoundlessAudit,
            "check_boundless_audit",
        ),
        (
            AntiPatternCode::ImplicitSubstrate,
            "check_implicit_substrate",
        ),
        (
            AntiPatternCode::AnchoringOverextension,
            "check_anchoring_overextension",
        ),
        (
            AntiPatternCode::SelfReferenceWithoutOperator,
            "check_self_reference_without_operator",
        ),
    ];
    assert_eq!(
        expected_fns.len(),
        40,
        "Expected mapping must cover all 40 AntiPatternCode variants"
    );

    for (code, fn_name) in expected_fns {
        let needle = format!("pub fn {}", fn_name);
        assert!(
            kernel_arch_anti_pattern_src.contains(&needle),
            "AntiPatternCode::{:?} ({}) has no `{}` implementation",
            code,
            code.code(),
            fn_name,
        );
    }
}

#[test]
fn pin_red_team_check_functions_present() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("arch_anti_pattern.rs"),
    )
    .expect("kernel source readable");
    for needle in &[
        "pub fn check_capability_ontology_v",
        "pub fn check_theorem_cve_required_v",
        "pub fn check_yoneda_canonical_roster_complete_v",
        "pub fn check_consumes_format_v",
    ] {
        assert!(
            src.contains(needle),
            "Red-team closure check missing: {} (AT-1..AT-5 require all four)",
            needle,
        );
    }
}

#[test]
fn pin_typed_attribute_parsers_present() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("arch_parse.rs"),
    )
    .expect("kernel source readable");
    for needle in &[
        "pub fn parse_arch_module",
        "pub fn parse_bridge_tier",
        "pub fn parse_deterministic",
        "pub fn parse_mtac_decision",
        "pub fn parse_arch_corpus",
    ] {
        assert!(
            src.contains(needle),
            "Typed-attribute parser missing: {}",
            needle,
        );
    }
}

#[test]
fn pin_auxiliary_attribute_types_present() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("arch.rs"),
    )
    .expect("kernel source readable");
    for needle in &[
        "pub struct BridgeTier",
        "pub struct DeterministicMarker",
        "pub struct MtacDecisionAttr",
        "pub struct ArchCorpusAttr",
        "pub enum MtacModality",
    ] {
        assert!(
            src.contains(needle),
            "Auxiliary attribute type missing: {}",
            needle,
        );
    }
}

// =============================================================================
// 15. Every core/math/*.vr file must carry @arch_module attestation
// =============================================================================

#[test]
fn pin_math_cogs_have_arch_module() {
    // ATS-V annotation discipline: every cog in `core/math/`
    // self-attests via `@arch_module(foundation, stratum,
    // lifecycle)`.  This pin reads each `.vr` file directly under
    // core/math/ and asserts it contains the attribute.  Files in
    // sub-directories (frameworks/, foundations/) are checked by
    // their own pins.
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolvable")
        .to_path_buf();
    let math_dir = workspace_root.join("core").join("math");

    let mut missing: Vec<String> = Vec::new();
    let mut total: usize = 0;
    for entry in std::fs::read_dir(&math_dir).expect("read core/math/") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("vr") {
            continue;
        }
        total += 1;
        let text = std::fs::read_to_string(&path).expect("read .vr");
        if !text.contains("@arch_module") {
            missing.push(
                path.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("<unknown>")
                    .to_string(),
            );
        }
    }
    assert!(
        missing.is_empty(),
        "{} of {} core/math/*.vr files missing @arch_module attestation: {}",
        missing.len(),
        total,
        missing.join(", "),
    );
    assert!(total >= 60, "core/math/ should have >= 60 .vr files, found {}", total);
}

// =============================================================================
// 16. registry.vr populates every framework mod.vr mounts
// =============================================================================

#[test]
fn pin_registry_covers_mod_mounts() {
    // The `frameworks/registry.vr` populators
    // (`populate_full_canonical` aggregating
    // `populate_canonical_standard` + `populate_diakrisis_extensions`
    // + `populate_msfs_catalogue` + `populate_bounded_arithmetic_family`
    // + `populate_experimental`) must register every public
    // `mount core.math.frameworks.X` in `frameworks/mod.vr`.
    //
    // This pin reads BOTH files as text and asserts:
    //   1. Every mount target name in mod.vr appears as a
    //      registered framework name in registry.vr.
    //   2. The expected_full_canonical_count() advertised total
    //      lines up with the literal-string count of
    //      `framework_record_new` invocations.
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolvable")
        .to_path_buf();
    let frameworks_dir = workspace_root.join("core").join("math").join("frameworks");

    let mod_text = std::fs::read_to_string(frameworks_dir.join("mod.vr"))
        .expect("read frameworks/mod.vr");
    let registry_text = std::fs::read_to_string(frameworks_dir.join("registry.vr"))
        .expect("read frameworks/registry.vr");

    // Extract the simple mount targets (single-segment after
    // `core.math.frameworks.`).  We accept the file-or-directory
    // names that `mod.vr` currently mounts; sub-mounts within
    // those subdirs are recorded with name expansions in the
    // registry (e.g. `msfs/{baseline,key_symbols,...}` →
    // `msfs_baseline`, etc.).
    let canonical_simple = [
        "lurie_htt",
        "schreiber_dcct",
        "connes_reconstruction",
        "petz_classification",
        "arnold_catastrophe",
        "baez_dolan",
        "owl2_fs",
        "diakrisis",
        "diakrisis_biadjunction",
        "diakrisis_stack_model",
        "diakrisis_extensions",
        "diakrisis_acts",
        "bounded_arithmetic",
        "registry",
        "msfs",
    ];
    for n in &canonical_simple {
        let needle = format!("core.math.frameworks.{}", n);
        assert!(
            mod_text.contains(&needle),
            "frameworks/mod.vr missing mount: {}",
            needle,
        );
    }

    // Verify each registered framework name we claim above appears
    // in registry.vr (foundational impl + citation pack + extensions).
    let registered_names = [
        // Standard tier — citation packages
        "\"lurie_htt\"",
        "\"schreiber_dcct\"",
        "\"connes_reconstruction\"",
        "\"petz_classification\"",
        "\"arnold_catastrophe\"",
        "\"baez_dolan\"",
        "\"owl2_fs\"",
        // Standard tier — meta-classifier + special
        "\"diakrisis\"",
        "\"actic.raw\"",
        // Standard tier — foundational implementations
        "\"zfc_two_inacc\"",
        "\"hott\"",
        "\"cubical\"",
        "\"mltt\"",
        "\"cic\"",
        "\"eff\"",
        // VerifiedExtension — diakrisis extensions
        "\"diakrisis_biadjunction\"",
        "\"diakrisis_stack_model\"",
        "\"diakrisis_extensions\"",
        "\"diakrisis_acts\"",
        // VerifiedExtension — MSFS catalogue
        "\"msfs_baseline\"",
        "\"msfs_key_symbols\"",
        "\"msfs_self_containment\"",
        "\"msfs_strcat\"",
        // VerifiedExtension — bounded-arithmetic family
        "\"bounded_arithmetic_v_0\"",
        "\"bounded_arithmetic_v_1\"",
        "\"bounded_arithmetic_s_2_1\"",
        "\"bounded_arithmetic_v_np\"",
        "\"bounded_arithmetic_v_ph\"",
        "\"bounded_arithmetic_i_delta_0\"",
    ];
    for n in &registered_names {
        assert!(
            registry_text.contains(n),
            "registry.vr missing framework_record_new(...) registration for {}",
            n,
        );
    }

    // Count `framework_record_new(` invocations and assert it
    // matches the advertised expected_full_canonical_count().
    let _invocations = registry_text.matches("framework_record_new(").count();
    // mod.vr declares 1 schema definition + 31 records.  Net
    // record-count is invocations - 1 (the type-constructor signature).
    // Simpler: count actual call sites by looking for the pattern
    // "registry_register(r, framework_record_new(" which is the
    // canonical invocation form.
    let registered = registry_text
        .matches("registry_register(r, framework_record_new(")
        .count();
    assert_eq!(
        registered, 29,
        "registry.vr should register exactly 29 frameworks (Standard 15 + VerifiedExt 14); \
         counted {} registry_register(r, framework_record_new(...)) sites",
        registered,
    );
    assert!(
        registry_text.contains("public fn populate_full_canonical("),
        "registry.vr must expose populate_full_canonical aggregator"
    );
    assert!(
        registry_text.contains("public fn expected_full_canonical_count()"),
        "registry.vr must expose expected_full_canonical_count"
    );
}

// =============================================================================
// 17. Capability ontology — kernel registry mirrors Verum-side roster
// =============================================================================

#[test]
fn pin_capability_ontology_aligned() {
    // Cross-side pin: kernel-static
    // `arch::canonical_capability_registry()` mirrors the Verum-side
    // `core/architecture/capability_ontology.vr::ATS_V_CANONICAL_CAPABILITIES`
    // list.  Adding a new canonical capability requires updating both
    // sides AND this pin in the same change-set.
    let kernel_registry = verum_kernel::arch::canonical_capability_registry();
    assert_eq!(
        kernel_registry.len(),
        7,
        "kernel canonical capability registry size pinned to 7"
    );
    let expected: std::collections::BTreeSet<&'static str> = [
        "logger",
        "metrics",
        "tracing",
        "config_read",
        "config_admin",
        "supervisor_spawn",
        "kernel_intrinsic",
    ]
    .iter()
    .copied()
    .collect();
    let actual: std::collections::BTreeSet<&str> =
        kernel_registry.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        actual, expected,
        "kernel canonical capability registry tag set drifted from canonical roster"
    );

    // Verify the Verum-side .vr surface has the same tags.
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolvable")
        .to_path_buf();
    let vr_text = std::fs::read_to_string(
        workspace_root
            .join("core")
            .join("architecture")
            .join("capability_ontology.vr"),
    )
    .expect("read capability_ontology.vr");
    for tag in &expected {
        let needle = format!("name: \"{}\"", tag);
        assert!(
            vr_text.contains(&needle),
            "Verum-side capability_ontology.vr missing canonical tag: {}",
            tag,
        );
    }
}

// =============================================================================
// 18. PhaseInputs — red-team data wiring exists on the kernel surface
// =============================================================================

#[test]
fn pin_phase_inputs_wires_red_team_data() {
    // The kernel `arch_phase::run_arch_phase_one_with` accepts a
    // `PhaseInputs` struct that propagates capability_ontology_registry,
    // yoneda_verdicts_claimed, and foreign_foundation_constructs into
    // the DiagnosticContext that drives `check_all_anti_patterns`.
    //
    // Without this wiring the red-team closures (AT-1 / AT-3 /
    // AP-026) would only fire in unit tests and never against
    // real cogs — silent regression risk.  This pin asserts the
    // PhaseInputs surface exists.
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("arch_phase.rs"),
    )
    .expect("read arch_phase.rs");
    assert!(
        src.contains("pub struct PhaseInputs"),
        "arch_phase.rs must expose PhaseInputs struct"
    );
    assert!(
        src.contains("pub fn run_arch_phase_one_with"),
        "arch_phase.rs must expose run_arch_phase_one_with entry"
    );
    assert!(
        src.contains("ctx.capability_ontology_registry"),
        "run_arch_phase_one* must populate ctx.capability_ontology_registry"
    );
    assert!(
        src.contains("canonical_capability_registry"),
        "run_arch_phase_one default must use canonical_capability_registry"
    );
}

// =============================================================================
// 19. Counterfactual / adjunction / yoneda operationalisation pin
// =============================================================================

#[test]
fn pin_counterfactual_helpers_present() {
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolvable")
        .to_path_buf();
    let cf = std::fs::read_to_string(
        workspace_root
            .join("core")
            .join("architecture")
            .join("counterfactual.vr"),
    )
    .expect("read counterfactual.vr");
    for needle in &[
        "public fn arch_metric_tag",
        "public fn metric_value_tag",
        "public fn invariant_status_tag",
        "public fn invariant_status_is_stable",
        "public fn report_overall_stable_predicate",
        "public fn invariant_status_uniqueness_pin",
        "public fn empty_invariants_unstable_pin",
    ] {
        assert!(
            cf.contains(needle),
            "counterfactual.vr missing operationalisation helper: {}",
            needle,
        );
    }
}

#[test]
fn pin_adjunction_helpers_present() {
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolvable")
        .to_path_buf();
    let adj = std::fs::read_to_string(
        workspace_root
            .join("core")
            .join("architecture")
            .join("adjunction.vr"),
    )
    .expect("read adjunction.vr");
    for needle in &[
        "public fn canonical_adjunction_tag",
        "public fn refactoring_direction_tag",
        "public fn adjunction_verdict_tag",
        "public fn adjunction_verdict_is_accepted",
        "public fn all_preservation_holds",
        "public fn all_gain_holds",
        "public fn chain_acceptance_predicate",
        "public fn verdict_acceptance_uniqueness_pin",
        "public fn empty_chain_rejected_pin",
    ] {
        assert!(
            adj.contains(needle),
            "adjunction.vr missing operationalisation helper: {}",
            needle,
        );
    }
}

#[test]
fn pin_yoneda_helpers_present() {
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolvable")
        .to_path_buf();
    let yon = std::fs::read_to_string(
        workspace_root
            .join("core")
            .join("architecture")
            .join("yoneda.vr"),
    )
    .expect("read yoneda.vr");
    for needle in &[
        "public fn observation_observer_tag",
        "public fn agreement_status_tag",
        "public fn all_agreements_agree",
        "public fn count_disagreements",
        "public fn yoneda_verdict_equivalent_predicate",
        "public fn empty_agreements_not_equivalent_pin",
        "public fn agreement_status_disjoint_pin",
    ] {
        assert!(
            yon.contains(needle),
            "yoneda.vr missing operationalisation helper: {}",
            needle,
        );
    }
}

// =============================================================================
// 20. @arch_module discipline extends to core/verify/ and core/proof/
// =============================================================================

#[test]
fn pin_verify_cogs_have_arch_module() {
    pin_dir_arch_module_coverage("verify", 5);
}

#[test]
fn pin_proof_cogs_have_arch_module() {
    pin_dir_arch_module_coverage("proof", 5);
}

/// Helper: every `.vr` file directly under `core/<dir>/` must
/// carry `@arch_module(...)` self-attestation.
fn pin_dir_arch_module_coverage(dir_name: &str, expected_min: usize) {
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolvable")
        .to_path_buf();
    let target = workspace_root.join("core").join(dir_name);

    let mut missing: Vec<String> = Vec::new();
    let mut total: usize = 0;
    for entry in std::fs::read_dir(&target).expect("read core/<dir>/") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("vr") {
            continue;
        }
        total += 1;
        let text = std::fs::read_to_string(&path).expect("read .vr");
        if !text.contains("@arch_module") {
            missing.push(
                path.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("<unknown>")
                    .to_string(),
            );
        }
    }
    assert!(
        missing.is_empty(),
        "{} of {} core/{}/*.vr files missing @arch_module attestation: {}",
        missing.len(),
        total,
        dir_name,
        missing.join(", "),
    );
    assert!(
        total >= expected_min,
        "core/{}/ should have >= {} .vr files, found {}",
        dir_name,
        expected_min,
        total
    );
}

// =============================================================================
// 21. Compiler ats_v_phase wires foreign-foundation citations
// =============================================================================

#[test]
fn pin_compiler_phase_wires_foreign_foundation_constructs() {
    // Phase M closure: verum_compiler::pipeline::ats_v_phase calls
    // run_arch_phase_one_with (not bare run_arch_phase_one) and
    // surfaces @framework(corpus, "...") body annotations into
    // PhaseInputs.foreign_foundation_constructs so AP-026
    // FoundationContentMismatch fires in real builds.
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolvable")
        .to_path_buf();
    let phase_src = std::fs::read_to_string(
        workspace_root
            .join("crates")
            .join("verum_compiler")
            .join("src")
            .join("pipeline")
            .join("ats_v_phase.rs"),
    )
    .expect("read ats_v_phase.rs");
    assert!(
        phase_src.contains("run_arch_phase_one_with"),
        "ats_v_phase.rs must call run_arch_phase_one_with (not bare run_arch_phase_one)"
    );
    assert!(
        phase_src.contains("PhaseInputs"),
        "ats_v_phase.rs must construct PhaseInputs"
    );
    assert!(
        phase_src.contains("extract_foreign_foundation_constructs"),
        "ats_v_phase.rs must extract foreign-foundation citations"
    );
}

// =============================================================================
// 22. Universal @arch_module discipline across the entire core/ stdlib
// =============================================================================

/// **Universal pin** — every `.vr` file under `core/` (recursive)
/// must carry the `@arch_module(...)` self-attestation declaration.
/// This is the architectural promise that ATS-V annotation
/// discipline applies UNIFORMLY across the stdlib, not just the
/// math / verify / proof / architecture sub-trees.
///
/// Files exempt from the pin (very narrow exception list):
///   * Files with no `module core.X.Y;` declaration are not cogs
///     in the Verum sense — they are auxiliary helper files (test
///     fixtures, generated stubs).  None currently exist in core/
///     but the pin allows the future case.
///
/// Adding a new `.vr` cog requires either annotating it with
/// `@arch_module(...)` OR adding an explicit exemption with a
/// rationale comment.
#[test]
fn pin_universal_arch_module_coverage_in_core() {
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolvable")
        .to_path_buf();
    let core = workspace_root.join("core");

    let mut total_cogs: usize = 0;
    let mut missing: Vec<String> = Vec::new();
    walk_core_vr_files(&core, &mut |path| {
        let text = std::fs::read_to_string(path).expect("read .vr");
        // A file qualifies as a cog iff it contains `^module core.X.Y;`.
        // Files without that declaration are auxiliary (none currently).
        let mut module_lines = 0;
        for line in text.lines() {
            if line.starts_with("module core") {
                module_lines += 1;
            }
        }
        if module_lines == 0 {
            return;
        }
        total_cogs += 1;
        if !text.contains("@arch_module") {
            missing.push(
                path.strip_prefix(&core)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| path.display().to_string()),
            );
        }
    });

    assert!(
        missing.is_empty(),
        "{} of {} cogs under core/ missing @arch_module attestation: {}",
        missing.len(),
        total_cogs,
        missing.join(", "),
    );
    // Sanity floor: stdlib has at least 1500 cogs.  If this drops
    // sharply something deleted a directory by accident.
    assert!(
        total_cogs >= 1500,
        "core/ should contain >= 1500 annotated cogs, found {}",
        total_cogs,
    );
}

fn walk_core_vr_files(dir: &std::path::Path, f: &mut impl FnMut(&std::path::Path)) {
    if !dir.is_dir() {
        return;
    }
    // Skip target/ — build artifacts.
    if dir.file_name().and_then(|s| s.to_str()) == Some("target") {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_core_vr_files(&path, f);
        } else if path.extension().and_then(|s| s.to_str()) == Some("vr") {
            f(&path);
        }
    }
}

// =============================================================================
// 23. Audit-bundle CLI walks all 16 ATS-V intrinsics
// =============================================================================

#[test]
fn pin_audit_bundle_walks_all_ats_v_intrinsics() {
    // The verum_cli `verum audit --arch-discharges` gate iterates
    // a static `arch_intrinsics` list and dispatches each entry.
    // This pin ensures the list covers all 16 ATS-V intrinsics:
    //   * 8 base (capability/boundary/composition/lifecycle/foundation/
    //     anti_pattern/cve_closure/soundness_v0)
    //   * 4 surface (mtac/counterfactual/adjunction/yoneda)
    //   * 4 operational engine (composition_engine/_associative/
    //     corpus_verify/phase_orchestrator)
    //   * 4 red-team (capability_ontology/yoneda_canonical_roster/
    //     theorem_cve_required/consumes_format)
    //
    // = 16 unique intrinsic names.  Adding a new ATS-V intrinsic
    // requires extending the audit-bundle list AND this pin in
    // the same change-set.
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolvable")
        .to_path_buf();
    let audit_src = std::fs::read_to_string(
        workspace_root
            .join("crates")
            .join("verum_cli")
            .join("src")
            .join("commands")
            .join("audit.rs"),
    )
    .expect("read audit.rs");

    let expected_ats_v_intrinsics: &[&str] = &[
        // Base 8.
        "kernel_arch_capability_discipline",
        "kernel_arch_boundary_check",
        "kernel_arch_composition_check",
        "kernel_arch_lifecycle_check",
        "kernel_arch_foundation_consistency",
        "kernel_arch_anti_pattern_check",
        "kernel_arch_cve_closure",
        "kernel_arch_soundness_v0",
        // Surface 4.
        "kernel_arch_mtac_calculus",
        "kernel_arch_counterfactual_engine",
        "kernel_arch_adjunction_analyzer",
        "kernel_arch_yoneda_equivalence",
        // Operational engine 4.
        "kernel_arch_composition_engine",
        "kernel_arch_composition_associative",
        "kernel_arch_corpus_verify",
        "kernel_arch_phase_orchestrator",
        // Red-team 4.
        "kernel_arch_capability_ontology_check",
        "kernel_arch_yoneda_canonical_roster_complete",
        "kernel_arch_theorem_cve_required",
        "kernel_arch_consumes_format_check",
    ];
    assert_eq!(
        expected_ats_v_intrinsics.len(),
        20,
        "expected total ATS-V intrinsic count drifted (was 8 base + 4 surface + 4 engine + 4 red-team = 20; \
         the 'archive 16' name in earlier docs counts only the 12 dispatcher + 4 red-team)"
    );

    for intrinsic in expected_ats_v_intrinsics {
        let needle = format!("\"{}\"", intrinsic);
        assert!(
            audit_src.contains(&needle),
            "audit.rs arch_intrinsics list missing entry: {}",
            intrinsic,
        );
    }
}

// =============================================================================
// 24. Q4 cross-cog peer resolution wiring
// =============================================================================

#[test]
fn pin_phase_inputs_cross_cog_fields_present() {
    // PhaseInputs gained 3 cross-cog slots in Q4:
    // composed_foundations / cited_lifecycles / callee_tiers.
    // These activate AP-005 / AP-009 / AP-004 when the compiler
    // resolves peers from the session-level arch-shape registry.
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("arch_phase.rs"),
    )
    .expect("read arch_phase.rs");
    for needle in &[
        "pub composed_foundations:",
        "pub cited_lifecycles:",
        "pub callee_tiers:",
    ] {
        assert!(
            src.contains(needle),
            "PhaseInputs missing cross-cog field: {}",
            needle,
        );
    }
    for needle in &[
        "ctx.composed_foundations = inputs.composed_foundations.clone()",
        "ctx.cited_lifecycles = inputs.cited_lifecycles.clone()",
        "ctx.callee_tiers = inputs.callee_tiers.clone()",
    ] {
        assert!(
            src.contains(needle),
            "run_arch_phase_one_with not propagating cross-cog field: {}",
            needle,
        );
    }
}

#[test]
fn pin_compiler_session_arch_shape_registry_present() {
    // Q4 requires the compiler-side Session to maintain a
    // per-module arch-shape registry; phase_ats_v populates it
    // and resolves peers' Foundation/Lifecycle/Tier from it.
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolvable")
        .to_path_buf();
    let session_src = std::fs::read_to_string(
        workspace_root
            .join("crates")
            .join("verum_compiler")
            .join("src")
            .join("session.rs"),
    )
    .expect("read session.rs");
    for needle in &[
        "arch_shape_registry:",
        "pub fn register_arch_shape",
        "pub fn resolve_composed_foundations",
        "pub fn resolve_cited_lifecycles",
        "pub fn resolve_callee_tiers",
    ] {
        assert!(
            session_src.contains(needle),
            "session.rs missing arch-shape registry surface: {}",
            needle,
        );
    }

    let phase_src = std::fs::read_to_string(
        workspace_root
            .join("crates")
            .join("verum_compiler")
            .join("src")
            .join("pipeline")
            .join("ats_v_phase.rs"),
    )
    .expect("read ats_v_phase.rs");
    for needle in &[
        "run_arch_phase_for_attrs_registry_aware",
        ".register_arch_shape(",
        ".resolve_composed_foundations(",
        ".resolve_cited_lifecycles(",
        ".resolve_callee_tiers(",
    ] {
        assert!(
            phase_src.contains(needle),
            "ats_v_phase.rs missing registry-aware wiring: {}",
            needle,
        );
    }
}

// =============================================================================
// 25. Q2 — module-wide @framework citation aggregation
// =============================================================================

#[test]
fn pin_compiler_phase_aggregates_module_wide_framework_citations() {
    // Q2: AP-026 FoundationContentMismatch needs to see citations
    // from EVERY item in the module body, not just citations on the
    // module-level @arch_module declaration.  The compiler's
    // ats_v_phase aggregates them via collect_module_wide_foreign_foundations
    // before running the module-level check.
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolvable")
        .to_path_buf();
    let phase_src = std::fs::read_to_string(
        workspace_root
            .join("crates")
            .join("verum_compiler")
            .join("src")
            .join("pipeline")
            .join("ats_v_phase.rs"),
    )
    .expect("read ats_v_phase.rs");
    assert!(
        phase_src.contains("fn collect_module_wide_foreign_foundations("),
        "ats_v_phase.rs missing collect_module_wide_foreign_foundations helper"
    );
    assert!(
        phase_src.contains("collect_module_wide_foreign_foundations(module)"),
        "ats_v_phase.rs phase_ats_v must call collect_module_wide_foreign_foundations"
    );
    assert!(
        phase_src.contains("for item in &module.items"),
        "ats_v_phase.rs collector must iterate module.items"
    );
}

// =============================================================================
// 26. Q5 — body-level capability inference wiring (AP-001 production)
// =============================================================================

#[test]
fn pin_capability_inference_ontology_present() {
    // The kernel-side capability ontology resolves primitive call
    // paths to Capability values.  AP-001 CapabilityEscalation
    // consumes the inferred set in production builds.
    let count = verum_kernel::arch_capability_inference::ontology_size();
    assert!(
        count >= 30,
        "capability inference ontology size pinned to >= 30 entries; got {}",
        count
    );

    // Sample a few load-bearing entries.
    let must_have_paths: &[&str] = &[
        "core.io.fs.read_file",
        "core.io.fs.write_file",
        "core.net.http.get",
        "core.net.tcp.connect",
        "core.net.tcp.listen",
        "core.shell.exec",
        "core.security.random.bytes",
        "core.metrics.counter",
        "core.tracing.span",
        "core.diagnostics.log",
    ];
    for p in must_have_paths {
        assert!(
            verum_kernel::arch_capability_inference::lookup_capability(p).is_some(),
            "ontology missing canonical entry: {}",
            p,
        );
    }

    // Unknown paths fall through silently — no false attribution.
    assert!(
        verum_kernel::arch_capability_inference::lookup_capability("not.a.path").is_none()
    );
}

#[test]
fn pin_phase_inputs_inferred_used_capabilities_present() {
    // PhaseInputs gained `inferred_used_capabilities` in Q5 to
    // carry the body-level capability inference result.  AP-001
    // CapabilityEscalation consumes ctx.inferred_used_capabilities
    // through check_capability_escalation.
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("arch_phase.rs"),
    )
    .expect("read arch_phase.rs");
    assert!(
        src.contains("pub inferred_used_capabilities:"),
        "PhaseInputs missing inferred_used_capabilities field"
    );
    assert!(
        src.contains("ctx.inferred_used_capabilities = inputs.inferred_used_capabilities.clone()"),
        "run_arch_phase_one_with not propagating inferred_used_capabilities into ctx"
    );
}

#[test]
fn pin_compiler_phase_walks_body_for_capability_inference() {
    // Q5 walker lives in compiler-side ats_v_phase.rs.  This pin
    // asserts the walker exists and is invoked from phase_ats_v
    // for both module-level and per-item paths.
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolvable")
        .to_path_buf();
    let phase_src = std::fs::read_to_string(
        workspace_root
            .join("crates")
            .join("verum_compiler")
            .join("src")
            .join("pipeline")
            .join("ats_v_phase.rs"),
    )
    .expect("read ats_v_phase.rs");

    for needle in &[
        // The two entry points (module-wide + per-item).
        "pub(crate) fn infer_used_capabilities(",
        "pub(crate) fn infer_used_capabilities_in_item(",
        // The recursive walkers.
        "fn walk_item_body_for_caps(",
        "fn walk_block_for_caps(",
        "fn walk_stmt_for_caps(",
        "fn walk_expr_for_caps(",
        // The path resolver.
        "fn expr_to_dotted_path(",
        // The ontology dispatch site.
        "verum_kernel::arch_capability_inference::lookup_capability(",
        // Wired into phase_ats_v.
        "infer_used_capabilities(module)",
        "infer_used_capabilities_in_item(item)",
    ] {
        assert!(
            phase_src.contains(needle),
            "ats_v_phase.rs missing capability-inference surface: {}",
            needle,
        );
    }
}

// =============================================================================
// 27. R-AB-CD — transitive peer-graph walker (AP-019 / AP-024)
// =============================================================================

#[test]
fn pin_transitive_walker_present() {
    // R-A: kernel-side DFS infrastructure for multi-hop checks.
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("arch_transitive.rs"),
    )
    .expect("read arch_transitive.rs");
    for needle in &[
        "pub fn for_each_transitive_peer",
        "pub fn resolve_transitive_lifecycle_regressions",
        "pub fn resolve_transitive_foundation_downgrades",
        "pub const MAX_TRANSITIVE_DEPTH",
        "pub struct PeerVisit",
    ] {
        assert!(
            src.contains(needle),
            "arch_transitive.rs missing surface: {}",
            needle,
        );
    }
}

#[test]
fn pin_phase_inputs_transitive_fields_present() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("arch_phase.rs"),
    )
    .expect("read arch_phase.rs");
    for needle in &[
        "pub transitive_lifecycle_regressions:",
        "pub foundation_downgrades:",
        "ctx.transitive_lifecycle_regressions = inputs.transitive_lifecycle_regressions.clone()",
        "ctx.foundation_downgrades = inputs.foundation_downgrades.clone()",
    ] {
        assert!(
            src.contains(needle),
            "arch_phase.rs missing transitive wiring: {}",
            needle,
        );
    }
}

#[test]
fn pin_session_transitive_resolvers_present() {
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolvable")
        .to_path_buf();
    let session_src = std::fs::read_to_string(
        workspace_root
            .join("crates")
            .join("verum_compiler")
            .join("src")
            .join("session.rs"),
    )
    .expect("read session.rs");
    for needle in &[
        "pub fn resolve_transitive_lifecycle_regressions",
        "pub fn resolve_foundation_downgrades",
    ] {
        assert!(
            session_src.contains(needle),
            "session.rs missing transitive resolver: {}",
            needle,
        );
    }
    let phase_src = std::fs::read_to_string(
        workspace_root
            .join("crates")
            .join("verum_compiler")
            .join("src")
            .join("pipeline")
            .join("ats_v_phase.rs"),
    )
    .expect("read ats_v_phase.rs");
    for needle in &[
        ".resolve_transitive_lifecycle_regressions(",
        ".resolve_foundation_downgrades(",
        "transitive_lifecycle_regressions,",
        "foundation_downgrades,",
    ] {
        assert!(
            phase_src.contains(needle),
            "ats_v_phase.rs missing transitive wiring: {}",
            needle,
        );
    }
}

#[test]
fn pin_transitive_resolver_correctness() {
    // Direct correctness check using the resolver against a small
    // crafted registry.  This duplicates the kernel-internal unit
    // tests but locks the public API surface.
    use std::collections::BTreeMap;
    use verum_kernel::arch::*;
    use verum_kernel::arch_transitive::resolve_transitive_lifecycle_regressions;

    let mut registry: BTreeMap<String, Shape> = BTreeMap::new();
    let theorem_shape = |composes_with: Vec<String>| Shape {
        exposes: vec![],
        requires: vec![],
        preserves: vec![],
        consumes: vec![],
        at_tier: Tier::Aot,
        foundation: Foundation::ZfcTwoInacc,
        stratum: MsfsStratum::LFnd,
        cve_closure: CveClosure {
            constructive: None,
            verifiable_strategy: None,
            executable: None,
        },
        lifecycle: Lifecycle::Theorem {
            since: "v0.1".into(),
        },
        composes_with,
        strict: false,
        declarations: None,
    };
    let mut hypothesis_shape = theorem_shape(vec![]);
    hypothesis_shape.lifecycle = Lifecycle::Hypothesis {
        confidence: ConfidenceLevel::Low,
    };

    registry.insert("start".into(), theorem_shape(vec!["A".into()]));
    registry.insert("A".into(), theorem_shape(vec!["B".into()]));
    registry.insert("B".into(), hypothesis_shape);

    let theorem_rank = Lifecycle::Theorem {
        since: "v".into(),
    }
    .rank();
    let regressions =
        resolve_transitive_lifecycle_regressions("start", theorem_rank, &registry);
    assert_eq!(
        regressions.len(),
        1,
        "expected exactly 1 transitive regression chain"
    );
    let (intermediate, terminal, _) = &regressions[0];
    assert_eq!(intermediate, "A");
    assert_eq!(terminal, "B");
}

// =============================================================================
// 28. Internal/ references must NOT appear in any architecture .vr file
// =============================================================================

#[test]
fn pin_no_internal_references_in_arch_vr() {
    // The user-visible `.vr` files in core/architecture/ must be
    // self-sufficient — no dangling references to `internal/specs/...`
    // or `internal/holon/...` paths.  Detailed exposition lives
    // inline; the website mirrors the same content.
    for file in &[
        "types.vr",
        "mtac.vr",
        "counterfactual.vr",
        "adjunction.vr",
        "yoneda.vr",
        "anti_patterns.vr",
        "capability_ontology.vr",
        "composition.vr",
        "corpus.vr",
        "phase.vr",
        "parse.vr",
        "mod.vr",
    ] {
        let vr = read_vr(file);
        assert!(
            !vr.contains("internal/specs"),
            "core/architecture/{} contains a forbidden reference to `internal/specs/...` — replace with detailed inline exposition",
            file,
        );
        assert!(
            !vr.contains("internal/holon"),
            "core/architecture/{} contains a forbidden reference to `internal/holon/...` — replace with detailed inline exposition",
            file,
        );
    }
}
