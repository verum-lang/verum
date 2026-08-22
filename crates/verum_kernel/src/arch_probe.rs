//! Executable counterexamples for the anti-pattern roster (T0841).
//!
//! The catalogue claims forty architectural anti-patterns. A claim of
//! that shape is worth exactly as much as its falsifiability, and
//! until this module existed the claim was unfalsifiable: the kernel's
//! `kernel_arch_*` discharge intrinsics answered `decision(true, "the
//! intrinsic is wired")` — a sanity stamp minted before the ATS-V
//! phase existed and never revisited after the phase went live. A
//! trusted kernel that reports verdicts it never computed is the one
//! defect a proof kernel may not have.
//!
//! ## What a probe is
//!
//! One probe per anti-pattern: a `Shape` plus a `DiagnosticContext`
//! crafted so that the LIVE driver — [`check_all_anti_patterns`], the
//! same entry point the phase calls — must report that pattern, and
//! only the clean baseline must leave it silent. Running a probe is
//! therefore an execution of the real checker, not a description of
//! it, and it carries **both controls**:
//!
//! * the **positive** control (violating input ⇒ the code is
//!   reported) fails the moment a check is deleted, unwired from the
//!   driver, or turned vacuous;
//! * the **negative** control (clean baseline ⇒ the code is absent)
//!   fails the moment a check degenerates into "always fires", which
//!   a positive control alone would happily accept.
//!
//! Discharge = both controls pass. That is the honest verdict a
//! kernel arm may return for "this pattern is enforced", and the
//! probe table is what makes forty separate claims separately
//! falsifiable.
//!
//! ## Why the probes live in the kernel
//!
//! They are not tests of the kernel; they are the kernel's evidence.
//! `intrinsic_dispatch` executes them to answer a discharge request,
//! so they must ship in the same crate and run in the same process as
//! the checks they exercise. A test suite that lived elsewhere would
//! prove the checks work in CI and prove nothing about the verdict a
//! build actually received.

use crate::arch::{
    Capability, CveClosure, ExecutabilitySense, Foundation, Lifecycle, MsfsStratum, Purpose,
    ResourceTag, Shape, ShapeDeclarations, Tier, VerifyStrategy,
};
use crate::arch_anti_pattern::{
    AntiPatternCode, DiagnosticContext, ForbiddenCitation, ForbiddenRegisterKind, ShapeDelta,
    check_all_anti_patterns,
};

/// The verdict of running one pattern's probe through the live driver.
///
/// Deliberately three-valued: "not discharged" is not one condition
/// but two opposite failures, and a diagnostic that conflated them
/// would send the reader looking in the wrong place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// Both controls passed: the violating input was flagged with
    /// this pattern's code and the clean baseline was not.
    Discharged,
    /// The violating input produced no such violation — the check is
    /// missing, unwired from the driver, or vacuous.
    CheckerSilent,
    /// The clean baseline produced this violation too — the check
    /// fires indiscriminately, so a hit proves nothing.
    CheckerIndiscriminate,
}

impl ProbeOutcome {
    /// True only for [`ProbeOutcome::Discharged`].
    pub fn is_discharged(self) -> bool {
        matches!(self, ProbeOutcome::Discharged)
    }

    /// One-line rationale suitable for a kernel `Decision` reason.
    pub fn rationale(self, code: AntiPatternCode) -> String {
        match self {
            ProbeOutcome::Discharged => format!(
                "{} {} — DISCHARGED by execution: the canonical counterexample is \
                 reported by the live anti-pattern driver and the clean baseline is not",
                code.code(),
                code.name(),
            ),
            ProbeOutcome::CheckerSilent => format!(
                "{} {} — NOT discharged: the canonical counterexample passed the live \
                 driver unreported. The check is missing, unwired from \
                 check_all_anti_patterns, or vacuous",
                code.code(),
                code.name(),
            ),
            ProbeOutcome::CheckerIndiscriminate => format!(
                "{} {} — NOT discharged: the clean baseline is reported as violating, so \
                 the check fires indiscriminately and a hit carries no information",
                code.code(),
                code.name(),
            ),
        }
    }
}

/// A canonical counterexample for one anti-pattern.
pub struct AntiPatternProbe {
    /// The pattern this probe exercises.
    pub code: AntiPatternCode,
    /// Builds the violating input. Kept as a function rather than a
    /// value because `Shape` is not `Copy` and a probe must hand a
    /// fresh, unmutated input to every run.
    pub violating: fn() -> (Shape, DiagnosticContext),
}

/// The clean baseline both controls are measured against: an
/// unannotated cog with an empty diagnostic context. Pinned by
/// `check_all_returns_empty_on_clean_default_shape` in the
/// anti-pattern suite — no check may fire on it.
fn clean_baseline() -> (Shape, DiagnosticContext) {
    (Shape::default_for_unannotated(), DiagnosticContext::default())
}

// ---------------------------------------------------------------------------
// Small constructors — keep each probe to its one distinguishing fact
// ---------------------------------------------------------------------------

fn cap_read_logger() -> Capability {
    Capability::Read {
        resource: ResourceTag::Logger,
    }
}

fn cap_write_logger() -> Capability {
    Capability::Write {
        resource: ResourceTag::Logger,
    }
}

fn theorem_lifecycle() -> Lifecycle {
    Lifecycle::Theorem {
        since: "probe".to_string(),
    }
}

/// Declarations with every CVE-articulation axis present. Each
/// strict-mode probe starts here and removes exactly ONE axis, so the
/// pattern it exercises is the only one it can trigger.
fn full_declarations() -> ShapeDeclarations {
    ShapeDeclarations {
        purpose: Some(Purpose {
            role: "probe".to_string(),
            k_min: crate::arch::CveThresholdK::TypedSchema,
            v_min: crate::arch::CveThresholdV::TypecheckPlusTests,
            e_min: crate::arch::CveThresholdE::StructurallyReady,
        }),
        substrate: Some(crate::arch::CognitiveSubstrate::AnalyticDecompositional),
        anchoring: Some(crate::arch::FormalAnchoring::CurryHowardLawvere),
        e_sense: Some(ExecutabilitySense::StructuralReadiness),
        self_reference: None,
    }
}

/// A strict shape carrying full declarations — the base every
/// strict-mode probe subtracts from.
fn strict_shape() -> Shape {
    let mut shape = Shape::default_for_unannotated();
    shape.strict = true;
    shape.declarations = Some(full_declarations());
    shape.cve_closure = CveClosure {
        constructive: Some("probe witness".to_string()),
        verifiable_strategy: Some(VerifyStrategy::Static),
        executable: Some("probe artefact".to_string()),
    };
    shape
}

// ---------------------------------------------------------------------------
// The probes
// ---------------------------------------------------------------------------

fn probe_capability_escalation() -> (Shape, DiagnosticContext) {
    let mut shape = Shape::default_for_unannotated();
    shape.requires = vec![cap_read_logger()];
    let ctx = DiagnosticContext {
        // Writing to a resource the cog only declared a read for.
        inferred_used_capabilities: vec![cap_read_logger(), cap_write_logger()],
        ..Default::default()
    };
    (shape, ctx)
}

fn probe_capability_leak() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        leaked_capabilities: vec![cap_write_logger()],
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_dependency_cycle() -> (Shape, DiagnosticContext) {
    let mut shape = Shape::default_for_unannotated();
    shape.composes_with = vec!["probe_cog".to_string()];
    let ctx = DiagnosticContext {
        cog_name: "probe_cog".to_string(),
        composes_graph: vec![("probe_cog".to_string(), vec!["probe_cog".to_string()])],
        ..Default::default()
    };
    (shape, ctx)
}

fn probe_tier_mixing() -> (Shape, DiagnosticContext) {
    let mut shape = Shape::default_for_unannotated();
    shape.at_tier = Tier::Interp;
    let ctx = DiagnosticContext {
        callee_tiers: vec![("gpu_callee".to_string(), Tier::Gpu)],
        ..Default::default()
    };
    (shape, ctx)
}

fn probe_foundation_drift() -> (Shape, DiagnosticContext) {
    let mut shape = Shape::default_for_unannotated();
    shape.foundation = Foundation::ZfcTwoInacc;
    let ctx = DiagnosticContext {
        composed_foundations: vec![("hott_cog".to_string(), Foundation::Hott)],
        ..Default::default()
    };
    (shape, ctx)
}

fn probe_register_mixing() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        forbidden_citations: vec![ForbiddenCitation {
            kind: ForbiddenRegisterKind::AuthoritativeAppeal,
            location: "probe.vr:1".to_string(),
            source: "as the authority states".to_string(),
        }],
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_tx_straddling() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        straddling_txs: vec!["tx_across_await".to_string()],
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_resource_straddling() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        straddling_resources: vec!["file_handle_across_await".to_string()],
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_lifecycle_regression() -> (Shape, DiagnosticContext) {
    let mut shape = Shape::default_for_unannotated();
    shape.lifecycle = theorem_lifecycle();
    let ctx = DiagnosticContext {
        // A proven artefact citing a speculative one.
        cited_lifecycles: vec![(
            "speculative_cog".to_string(),
            Lifecycle::Hypothesis {
                confidence: crate::arch::ConfidenceLevel::Low,
            },
        )],
        ..Default::default()
    };
    (shape, ctx)
}

fn probe_cve_incomplete() -> (Shape, DiagnosticContext) {
    let mut shape = strict_shape();
    shape.lifecycle = theorem_lifecycle();
    // A Theorem in strict mode with an open CVE triple.
    shape.cve_closure = CveClosure {
        constructive: None,
        verifiable_strategy: None,
        executable: None,
    };
    (shape, DiagnosticContext::default())
}

fn probe_absolute_boundary_attempt() -> (Shape, DiagnosticContext) {
    let mut shape = Shape::default_for_unannotated();
    shape.stratum = MsfsStratum::LAbs;
    (shape, DiagnosticContext::default())
}

fn probe_invariant_violation() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        boundary_invariant_status: vec![(
            "probe_boundary".to_string(),
            crate::arch::BoundaryInvariant::AllOrNothing,
            false,
        )],
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_dangling_message_type() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        dangling_message_types: vec!["UnroutedMessage".to_string()],
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_unauthenticated_crossing() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        network_boundaries_without_auth: vec!["public_ingress".to_string()],
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_deterministic_violation() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        deterministic_violations: vec![(
            "serialise".to_string(),
            "map iteration order".to_string(),
        )],
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_capability_duplication() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        linear_capability_duplications: vec![cap_write_logger()],
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_orphan_capability() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        relevant_capability_orphans: vec![cap_read_logger()],
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_missing_handoff() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        composition_handoff_gaps: vec![("downstream_cog".to_string(), cap_write_logger())],
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_foundation_downgrade() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        foundation_downgrades: vec![(
            "downgraded_cog".to_string(),
            Foundation::Cic,
            Foundation::Eff,
        )],
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_time_bound_leakage() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        time_bound_leaks: vec![cap_read_logger()],
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_persistence_mismatch() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        persistence_mismatches: vec![(cap_write_logger(), "ephemeral store".to_string())],
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_capability_laundering() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        // A re-export chain long enough that the capability's origin
        // is no longer visible at the consumer.
        capability_laundering_chain_length: 8,
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_foundation_forgery() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        foundation_forgeries: vec![(Foundation::Hott, "uses ZFC choice".to_string())],
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_transitive_lifecycle_regression() -> (Shape, DiagnosticContext) {
    let mut shape = Shape::default_for_unannotated();
    shape.lifecycle = theorem_lifecycle();
    let ctx = DiagnosticContext {
        transitive_lifecycle_regressions: vec![(
            "mid_cog".to_string(),
            "leaf_cog".to_string(),
            Lifecycle::Hypothesis {
                confidence: crate::arch::ConfidenceLevel::Low,
            },
        )],
        ..Default::default()
    };
    (shape, ctx)
}

fn probe_declaration_drift() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        declaration_drift: Some(ShapeDelta {
            missing_in_declared: vec![cap_write_logger()],
            missing_in_body: Vec::new(),
            summary: "body writes a capability the declaration omits".to_string(),
        }),
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_foundation_content_mismatch() -> (Shape, DiagnosticContext) {
    let mut shape = Shape::default_for_unannotated();
    shape.foundation = Foundation::Hott;
    let ctx = DiagnosticContext {
        foreign_foundation_constructs: vec![("excluded_middle".to_string(), Foundation::ZfcTwoInacc)],
        ..Default::default()
    };
    (shape, ctx)
}

fn probe_retracted_citation_use() -> (Shape, DiagnosticContext) {
    let mut shape = Shape::default_for_unannotated();
    shape.lifecycle = theorem_lifecycle();
    let ctx = DiagnosticContext {
        cited_lifecycles: vec![(
            "withdrawn_cog".to_string(),
            Lifecycle::Retracted {
                reason: "superseded by the probe".to_string(),
                replacement: None,
            },
        )],
        ..Default::default()
    };
    (shape, ctx)
}

fn probe_hypothesis_without_maturation_plan() -> (Shape, DiagnosticContext) {
    let mut shape = Shape::default_for_unannotated();
    shape.lifecycle = Lifecycle::Hypothesis {
        confidence: crate::arch::ConfidenceLevel::Medium,
    };
    let ctx = DiagnosticContext {
        has_plan_attribute: false,
        ..Default::default()
    };
    (shape, ctx)
}

fn probe_interpretation_in_mature_corpus() -> (Shape, DiagnosticContext) {
    let mut shape = Shape::default_for_unannotated();
    shape.lifecycle = Lifecycle::Interpretation {
        reason: "editorial reading".to_string(),
    };
    let ctx = DiagnosticContext {
        in_mature_corpus: true,
        ..Default::default()
    };
    (shape, ctx)
}

fn probe_observer_impersonation() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        observer_role_register_mismatches: vec![(
            "peer_cog".to_string(),
            "end_user".to_string(),
        )],
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_boundless_audit() -> (Shape, DiagnosticContext) {
    let mut shape = strict_shape();
    // Exactly one axis removed: the audit has no termination criterion.
    if let Some(decls) = shape.declarations.as_mut() {
        decls.purpose = None;
    }
    (shape, DiagnosticContext::default())
}

fn probe_implicit_substrate() -> (Shape, DiagnosticContext) {
    let mut shape = strict_shape();
    // The pattern is about a LAW-BEARING cog hiding its operational
    // mode, so the probe must be a Theorem.
    shape.lifecycle = theorem_lifecycle();
    if let Some(decls) = shape.declarations.as_mut() {
        decls.substrate = None;
    }
    (shape, DiagnosticContext::default())
}

fn probe_anchoring_overextension() -> (Shape, DiagnosticContext) {
    let mut shape = strict_shape();
    // Overextension is claiming an anchoring-free theorem OUTSIDE the
    // Curry-Howard-Lawvere domain, where the anchoring cannot be
    // inferred from the foundation and must be declared.
    shape.lifecycle = theorem_lifecycle();
    shape.foundation = Foundation::Custom {
        name: "probe_foundation".to_string(),
        framework_corpus: "probe_corpus".to_string(),
    };
    if let Some(decls) = shape.declarations.as_mut() {
        decls.anchoring = None;
    }
    (shape, DiagnosticContext::default())
}

fn probe_self_reference_without_operator() -> (Shape, DiagnosticContext) {
    let mut shape = strict_shape();
    // The cog composes with itself and declares no self-reference
    // witness, so its fixed point is asserted without an operator.
    shape.composes_with = vec!["self_referential_probe".to_string()];
    if let Some(decls) = shape.declarations.as_mut() {
        decls.self_reference = None;
    }
    let ctx = DiagnosticContext {
        self_module_path: "self_referential_probe".to_string(),
        ..Default::default()
    };
    (shape, ctx)
}

fn probe_temporal_inconsistency() -> (Shape, DiagnosticContext) {
    let mut early = Shape::default_for_unannotated();
    early.foundation = Foundation::ZfcTwoInacc;
    let mut late = Shape::default_for_unannotated();
    // Same cog, different foundation at a later sample: the shape
    // drifted without an declared evolution.
    late.foundation = Foundation::Hott;
    let ctx = DiagnosticContext {
        temporal_samples: vec![
            (crate::arch_mtac::TimePoint::Past(1), early),
            (crate::arch_mtac::TimePoint::Now, late),
        ],
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_counterfactual_brittleness() -> (Shape, DiagnosticContext) {
    let decision = crate::arch_mtac::Decision {
        name: "probe_decision".to_string(),
        options: Vec::new(),
        chosen: None,
        depends_on: Vec::new(),
    };
    let ctx = DiagnosticContext {
        counterfactual_pairs: vec![crate::arch_mtac::CounterfactualPair {
            name: "probe_pair".to_string(),
            base: decision.clone(),
            alternative: decision,
            // No invariant survives the alternative — brittle.
            stability_invariants: Vec::new(),
        }],
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_missed_adjoint() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        refactorings_without_adjoint: vec!["extract_module".to_string()],
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_universal_property_violation() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        claimed_universal_property: Some("initial object".to_string()),
        // Claimed without the uniqueness witness that makes it universal.
        uniqueness_witness: None,
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_phantom_evolution() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        declared_evolutions: vec![crate::arch_mtac::ArchEvolution {
            trigger: String::new(),
            expected_time: crate::arch_mtac::TimePoint::Future(1),
            cost_class: crate::arch_mtac::ComplexityClass::Rewrite,
            reversibility: crate::arch_mtac::Reversibility::Irreversible,
        }],
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

fn probe_yoneda_inequivalent_refactor() -> (Shape, DiagnosticContext) {
    let ctx = DiagnosticContext {
        yoneda_observer_diff: vec![(
            crate::arch_mtac::Observer::PeerCog {
                module_path: "probe.peer".to_string(),
            },
            // `false` = this observer can TELL the two architectures
            // apart, which is precisely inequivalence under Yoneda.
            false,
        )],
        ..Default::default()
    };
    (Shape::default_for_unannotated(), ctx)
}

/// The probe table — one entry per catalogue code, in roster order.
///
/// Roster completeness is a gate, not a convention: see
/// `every_roster_code_has_a_probe`.
pub fn probe_table() -> Vec<AntiPatternProbe> {
    use AntiPatternCode as C;
    vec![
        AntiPatternProbe { code: C::CapabilityEscalation, violating: probe_capability_escalation },
        AntiPatternProbe { code: C::CapabilityLeak, violating: probe_capability_leak },
        AntiPatternProbe { code: C::DependencyCycle, violating: probe_dependency_cycle },
        AntiPatternProbe { code: C::TierMixing, violating: probe_tier_mixing },
        AntiPatternProbe { code: C::FoundationDrift, violating: probe_foundation_drift },
        AntiPatternProbe { code: C::RegisterMixing, violating: probe_register_mixing },
        AntiPatternProbe { code: C::TxStraddling, violating: probe_tx_straddling },
        AntiPatternProbe { code: C::ResourceStraddling, violating: probe_resource_straddling },
        AntiPatternProbe { code: C::LifecycleRegression, violating: probe_lifecycle_regression },
        AntiPatternProbe { code: C::CveIncomplete, violating: probe_cve_incomplete },
        AntiPatternProbe {
            code: C::AbsoluteBoundaryAttempt,
            violating: probe_absolute_boundary_attempt,
        },
        AntiPatternProbe { code: C::InvariantViolation, violating: probe_invariant_violation },
        AntiPatternProbe { code: C::DanglingMessageType, violating: probe_dangling_message_type },
        AntiPatternProbe {
            code: C::UnauthenticatedCrossing,
            violating: probe_unauthenticated_crossing,
        },
        AntiPatternProbe {
            code: C::DeterministicViolation,
            violating: probe_deterministic_violation,
        },
        AntiPatternProbe {
            code: C::CapabilityDuplication,
            violating: probe_capability_duplication,
        },
        AntiPatternProbe { code: C::OrphanCapability, violating: probe_orphan_capability },
        AntiPatternProbe { code: C::MissingHandoff, violating: probe_missing_handoff },
        AntiPatternProbe { code: C::FoundationDowngrade, violating: probe_foundation_downgrade },
        AntiPatternProbe { code: C::TimeBoundLeakage, violating: probe_time_bound_leakage },
        AntiPatternProbe { code: C::PersistenceMismatch, violating: probe_persistence_mismatch },
        AntiPatternProbe { code: C::CapabilityLaundering, violating: probe_capability_laundering },
        AntiPatternProbe { code: C::FoundationForgery, violating: probe_foundation_forgery },
        AntiPatternProbe {
            code: C::TransitiveLifecycleRegression,
            violating: probe_transitive_lifecycle_regression,
        },
        AntiPatternProbe { code: C::DeclarationDrift, violating: probe_declaration_drift },
        AntiPatternProbe {
            code: C::FoundationContentMismatch,
            violating: probe_foundation_content_mismatch,
        },
        AntiPatternProbe {
            code: C::TemporalInconsistency,
            violating: probe_temporal_inconsistency,
        },
        AntiPatternProbe {
            code: C::CounterfactualBrittleness,
            violating: probe_counterfactual_brittleness,
        },
        AntiPatternProbe { code: C::MissedAdjoint, violating: probe_missed_adjoint },
        AntiPatternProbe {
            code: C::UniversalPropertyViolation,
            violating: probe_universal_property_violation,
        },
        AntiPatternProbe { code: C::PhantomEvolution, violating: probe_phantom_evolution },
        AntiPatternProbe {
            code: C::YonedaInequivalentRefactor,
            violating: probe_yoneda_inequivalent_refactor,
        },
        AntiPatternProbe { code: C::RetractedCitationUse, violating: probe_retracted_citation_use },
        AntiPatternProbe {
            code: C::HypothesisWithoutMaturationPlan,
            violating: probe_hypothesis_without_maturation_plan,
        },
        AntiPatternProbe {
            code: C::InterpretationInMatureCorpus,
            violating: probe_interpretation_in_mature_corpus,
        },
        AntiPatternProbe {
            code: C::ObserverImpersonation,
            violating: probe_observer_impersonation,
        },
        AntiPatternProbe { code: C::BoundlessAudit, violating: probe_boundless_audit },
        AntiPatternProbe { code: C::ImplicitSubstrate, violating: probe_implicit_substrate },
        AntiPatternProbe {
            code: C::AnchoringOverextension,
            violating: probe_anchoring_overextension,
        },
        AntiPatternProbe {
            code: C::SelfReferenceWithoutOperator,
            violating: probe_self_reference_without_operator,
        },
    ]
}

/// Execute one pattern's probe through the live driver.
///
/// Returns `None` when the roster code has no probe — a condition the
/// completeness gate makes impossible, surfaced here rather than
/// silently reported as a pass.
pub fn run_probe(code: AntiPatternCode) -> Option<ProbeOutcome> {
    let table = probe_table();
    let probe = table.iter().find(|p| p.code == code)?;

    let (shape, ctx) = (probe.violating)();
    let flagged = check_all_anti_patterns(&shape, &ctx)
        .iter()
        .any(|v| v.code == code);
    if !flagged {
        return Some(ProbeOutcome::CheckerSilent);
    }

    let (clean_shape, clean_ctx) = clean_baseline();
    let fires_on_clean = check_all_anti_patterns(&clean_shape, &clean_ctx)
        .iter()
        .any(|v| v.code == code);
    Some(if fires_on_clean {
        ProbeOutcome::CheckerIndiscriminate
    } else {
        ProbeOutcome::Discharged
    })
}

/// Execute the whole roster. Used by the composite kernel discharges
/// (`kernel_arch_anti_pattern_check`, `kernel_arch_soundness_v0`),
/// which may only claim the catalogue when every pattern in it is
/// separately discharged.
pub fn run_all_probes() -> Vec<(AntiPatternCode, ProbeOutcome)> {
    probe_table()
        .iter()
        .map(|p| {
            let outcome = run_probe(p.code).unwrap_or(ProbeOutcome::CheckerSilent);
            (p.code, outcome)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Structural probes — evidence for the arms that claim an ALGEBRA
// rather than a pattern
// ---------------------------------------------------------------------------
//
// `kernel_arch_composition_*`, `kernel_arch_yoneda_equivalence`,
// `kernel_arch_corpus_verify` and `kernel_arch_phase_orchestrator` do
// not claim "pattern X is caught"; they claim that a piece of the
// architectural algebra behaves. Each is discharged the same way as a
// pattern probe — by executing the real implementation on a canonical
// input pair, positive and negative control together — so that a
// broken or vacuous implementation cannot be reported as sound.

/// Composition rejects an ill-formed pair and accepts a well-formed
/// one. `A ⊗ B` is well-formed only when B's requirements are met by
/// A's exposures; an implementation that composed anything, or
/// nothing, fails one of the two halves.
pub fn composition_algebra_holds() -> ProbeOutcome {
    use crate::arch_composition::compose;

    let mut producer = Shape::default_for_unannotated();
    producer.exposes = vec![cap_read_logger()];
    let mut consumer = Shape::default_for_unannotated();
    consumer.requires = vec![cap_read_logger()];
    if !compose(&producer, &consumer).is_composed() {
        // A pair whose capability flow IS satisfied must compose.
        return ProbeOutcome::CheckerIndiscriminate;
    }

    let mut starved = Shape::default_for_unannotated();
    starved.requires = vec![cap_write_logger()];
    if compose(&producer, &starved).is_composed() {
        // B requires a capability A never exposes — composing it
        // anyway is the failure this check exists to prevent.
        return ProbeOutcome::CheckerSilent;
    }
    ProbeOutcome::Discharged
}

/// Composition is associative on a canonical triple: `(A ⊗ B) ⊗ C`
/// and `A ⊗ (B ⊗ C)` must both succeed and agree on the composed
/// shape's capability surface.
pub fn composition_is_associative() -> ProbeOutcome {
    use crate::arch_composition::{CompositionResult, compose};

    let mut a = Shape::default_for_unannotated();
    a.exposes = vec![cap_read_logger()];
    let mut b = Shape::default_for_unannotated();
    b.requires = vec![cap_read_logger()];
    b.exposes = vec![cap_write_logger()];
    let mut c = Shape::default_for_unannotated();
    c.requires = vec![cap_write_logger()];

    let left = match compose(&a, &b) {
        CompositionResult::Composed(ab) => compose(&ab, &c),
        CompositionResult::Rejected(_) => return ProbeOutcome::CheckerSilent,
    };
    let right = match compose(&b, &c) {
        CompositionResult::Composed(bc) => compose(&a, &bc),
        CompositionResult::Rejected(_) => return ProbeOutcome::CheckerSilent,
    };
    match (left, right) {
        (CompositionResult::Composed(l), CompositionResult::Composed(r)) => {
            if l.exposes == r.exposes && l.requires == r.requires {
                ProbeOutcome::Discharged
            } else {
                ProbeOutcome::CheckerSilent
            }
        }
        _ => ProbeOutcome::CheckerSilent,
    }
}

/// Yoneda: a shape is equivalent to itself under every canonical
/// observer, and a shape that differs in an OBSERVABLE way is not.
pub fn yoneda_equivalence_holds() -> ProbeOutcome {
    use crate::arch_yoneda::yoneda_equivalent;

    let mut base = Shape::default_for_unannotated();
    base.exposes = vec![cap_read_logger()];
    if !yoneda_equivalent(&base, &base.clone(), &[]).equivalent {
        // Identity must be an equivalence, or the relation is not one.
        return ProbeOutcome::CheckerSilent;
    }

    let mut altered = base.clone();
    altered.exposes = vec![cap_read_logger(), cap_write_logger()];
    if yoneda_equivalent(&base, &altered, &[]).equivalent {
        // A consumer CAN tell these apart: one exposes a write the
        // other does not. Calling them equivalent makes the verdict
        // vacuous.
        return ProbeOutcome::CheckerIndiscriminate;
    }
    ProbeOutcome::Discharged
}

/// Corpus invariants: a corpus with a dependency cycle is rejected,
/// a clean one is accepted.
pub fn corpus_invariants_hold() -> ProbeOutcome {
    use crate::arch_corpus::verify_corpus;

    let clean = vec![
        ("alpha".to_string(), Shape::default_for_unannotated()),
        ("beta".to_string(), Shape::default_for_unannotated()),
    ];
    if !verify_corpus(&clean).is_load_bearing() {
        return ProbeOutcome::CheckerIndiscriminate;
    }

    let mut cyclic_a = Shape::default_for_unannotated();
    cyclic_a.composes_with = vec!["beta".to_string()];
    let mut cyclic_b = Shape::default_for_unannotated();
    cyclic_b.composes_with = vec!["alpha".to_string()];
    let cyclic = vec![
        ("alpha".to_string(), cyclic_a),
        ("beta".to_string(), cyclic_b),
    ];
    if verify_corpus(&cyclic).is_load_bearing() {
        return ProbeOutcome::CheckerSilent;
    }
    ProbeOutcome::Discharged
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every catalogue code carries a probe. Adding a pattern to the
    /// roster without an executable counterexample would restore the
    /// exact condition this module exists to end: a claim no run can
    /// falsify.
    #[test]
    fn every_roster_code_has_a_probe() {
        let table = probe_table();
        for code in AntiPatternCode::full_list() {
            assert!(
                table.iter().any(|p| p.code == code),
                "roster code {} ({}) has no probe — its enforcement claim is unfalsifiable",
                code.code(),
                code.name(),
            );
        }
        assert_eq!(
            table.len(),
            AntiPatternCode::full_list().len(),
            "the probe table and the roster must have the same length — a probe for a \
             code that is not in the roster is dead weight"
        );
    }

    /// Every probe discharges: its counterexample is caught and the
    /// clean baseline is not. A failure here names a check that is
    /// missing, unwired from `check_all_anti_patterns`, vacuous, or
    /// indiscriminate — and names WHICH.
    #[test]
    fn every_probe_discharges_through_the_live_driver() {
        let failures: Vec<String> = run_all_probes()
            .into_iter()
            .filter(|(_, outcome)| !outcome.is_discharged())
            .map(|(code, outcome)| outcome.rationale(code))
            .collect();
        assert!(
            failures.is_empty(),
            "anti-pattern probes did not discharge:\n{}",
            failures.join("\n"),
        );
    }

    /// Each structural probe discharges: the algebra its arm claims
    /// actually behaves, on both the accepting and the rejecting side.
    #[test]
    fn structural_probes_discharge() {
        let checks: [(&str, ProbeOutcome); 4] = [
            ("composition capability flow", composition_algebra_holds()),
            ("composition associativity", composition_is_associative()),
            ("yoneda equivalence", yoneda_equivalence_holds()),
            ("corpus invariants", corpus_invariants_hold()),
        ];
        let failures: Vec<String> = checks
            .iter()
            .filter(|(_, o)| !o.is_discharged())
            .map(|(what, o)| format!("{what}: {o:?}"))
            .collect();
        assert!(
            failures.is_empty(),
            "structural probes did not discharge: {}",
            failures.join("; "),
        );
    }

    /// The clean baseline is clean: no check fires on an unannotated
    /// cog with an empty context. This is the negative control's own
    /// control — if the baseline were dirty, every probe's negative
    /// half would be measuring the wrong thing.
    #[test]
    fn the_clean_baseline_is_clean() {
        let (shape, ctx) = clean_baseline();
        let violations = check_all_anti_patterns(&shape, &ctx);
        assert!(
            violations.is_empty(),
            "the clean baseline must produce no violations; got: {:?}",
            violations.iter().map(|v| v.code.code()).collect::<Vec<_>>(),
        );
    }
}
