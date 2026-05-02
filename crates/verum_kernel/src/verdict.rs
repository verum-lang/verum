//! Canonical verification-verdict types — single source of truth.
//!
//! ## Architectural role
//!
//! Pre-this-module, "verification verdict" lived in five parallel
//! shapes:
//!
//! * `Result<(), CheckError>` returned by `proof_checker::Certificate::verify`.
//! * `KernelOutcome` returned by `KernelRegistry::verify_all`.
//! * `DifferentialReport` returned by `differential::run_differential_test`.
//! * `IntrinsicValue::Decision { holds, reason }` returned by intrinsic dispatch.
//! * `ad-hoc Result<bool, ...>` in audit-gate Plain/Json formatters.
//!
//! Each carrier had its own combination algebra, its own audit-gate
//! formatting, its own JSON schema. The bundle audit was a
//! conjunction of fourteen separate verdict shapes with no unified
//! algebra.
//!
//! This module ships the **canonical [`VerificationVerdict`] type**:
//! one sum-type that every verification mechanism in `verum_kernel`
//! converts to. Audit gates, bundle aggregator, ATS-V phase (when
//! it lands per `internal/specs/ats-v.md`) — all consume this single
//! type. No parallel verdict shapes; every redundancy in the
//! verification stack reduces to identity-on-this-type.
//!
//! ## Alignment with the ATS-V specification
//!
//! Per `internal/specs/ats-v.md` §17.1 (Reuse compliance audit), the
//! V-axis (verification) of every artifact reduces to the existing
//! `@verify(strategy)` ladder. This type's [`DischargeMethod`]
//! enum is the kernel-side mirror of that ladder, plus the broader
//! discharge surface (kernel intrinsics, framework citations,
//! differential agreement, MSFS corpus theorems) that exists outside
//! the pure SMT path.
//!
//! Future ATS-V discharge variants (CapabilityCheck, BoundaryCheck,
//! CompositionCheck — per spec §4) plug into [`DischargeMethod`] as
//! additional variants without disturbing the verdict algebra.
//!
//! ## Reuse rules
//!
//! Existing modules retain their per-domain types
//! (`KernelOutcome`, `DifferentialReport`, etc.) but provide
//! `Into<VerificationVerdict>` conversions. The canonical type
//! becomes the **interchange format** at API boundaries; per-domain
//! types live as construction conveniences.

use std::collections::BTreeMap;

// =============================================================================
// VerificationVerdict — the canonical sum
// =============================================================================

/// Canonical verification verdict.
///
/// **Soundness contract**: every value of `VerificationVerdict`
/// carries a [`DischargeMethod`] tag identifying which mechanism
/// produced the verdict. Audit reports MUST surface the method —
/// "discharged" is meaningless without the method that did the
/// discharging.
#[derive(Debug, Clone)]
pub enum VerificationVerdict {
 /// The artifact was discharged by `method`.
 /// `evidence` carries method-specific witness (proof term hash,
 /// SMT certificate, kernel intrinsic reason, etc.).
    Discharged {
        method: DischargeMethod,
        evidence: Evidence,
    },
 /// The method ran and rejected the artifact.
 /// `counterexample` carries method-specific failure data.
    Rejected {
        method: DischargeMethod,
        counterexample: Counterexample,
    },
 /// The method was attempted but did not produce a definite
 /// answer (timeout, UNKNOWN, IOU pending). Distinct from
 /// `Rejected`: `Pending` is "no answer", not "rejected".
    Pending {
        method: DischargeMethod,
        reason: PendingReason,
    },
 /// Multiple methods disagreed. The audit gate's failure mode:
 /// `accepting` and `rejecting` lists must both be non-empty
 /// for this variant.
    Conflicted {
        accepting: Vec<DischargeMethod>,
        rejecting: Vec<DischargeMethod>,
    },
}

impl VerificationVerdict {
 /// Stable diagnostic tag for audit-report rendering.
    pub fn tag(&self) -> &'static str {
        match self {
            VerificationVerdict::Discharged { .. } => "discharged",
            VerificationVerdict::Rejected { .. } => "rejected",
            VerificationVerdict::Pending { .. } => "pending",
            VerificationVerdict::Conflicted { .. } => "conflicted",
        }
    }

 /// True iff the verdict is a clean discharge. Audit gates
 /// dispatch on this for pass/fail.
    pub fn is_discharged(&self) -> bool {
        matches!(self, VerificationVerdict::Discharged { .. })
    }

 /// True iff the verdict is a clean rejection (the artifact
 /// is unsound under the method). Adversarial-corpus audits
 /// dispatch on this — they REQUIRE rejection.
    pub fn is_rejected(&self) -> bool {
        matches!(self, VerificationVerdict::Rejected { .. })
    }

 /// True iff the verdict is conflicted — load-bearing audit
 /// failure: methods disagree.
    pub fn is_conflicted(&self) -> bool {
        matches!(self, VerificationVerdict::Conflicted { .. })
    }

 /// True iff the verdict is pending (no definite answer).
    pub fn is_pending(&self) -> bool {
        matches!(self, VerificationVerdict::Pending { .. })
    }

 /// Method tag (Latin canonical name) when a single method
 /// produced the verdict; `None` for `Conflicted`.
    pub fn primary_method(&self) -> Option<&DischargeMethod> {
        match self {
            VerificationVerdict::Discharged { method, .. } => Some(method),
            VerificationVerdict::Rejected { method, .. } => Some(method),
            VerificationVerdict::Pending { method, .. } => Some(method),
            VerificationVerdict::Conflicted { .. } => None,
        }
    }
}

// =============================================================================
// DischargeMethod — every verification mechanism in verum_kernel
// =============================================================================

/// Every discharge mechanism the kernel recognises. Single
/// enumeration for all verification entry points.
///
/// The variant set is closed and stable: adding a new variant
/// requires kernel-version bump + audit-gate migration. Domain-
/// specific routes (refinement contracts, separation-logic encoder,
/// codegen attestation) reduce to one of these variants — the
/// kernel doesn't introduce per-subsystem method enums.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DischargeMethod {
 /// Z3 / CVC5 / portfolio SMT discharge — `@verify(formal)` and
 /// stronger ladder strategies.
    Smt { backend: SmtBackend },
 /// Per-kernel-impl checker (proof_checker, proof_checker_nbe,
 /// future Verum-self-hosted kernel via #154).
    KernelChecker { name: &'static str },
 /// Kernel intrinsic dispatch arm (e.g., `kernel_truncate_to_level`,
 /// `kernel_self_soundness_in_meta_universe`,
 /// `kernel_reflection_tower_*`). Name matches the
 /// `available_intrinsics()` roster.
    KernelIntrinsic { name: &'static str },
 /// `@framework(corpus, "citation")` route — trusted-boundary
 /// axiom citing published proof.
    FrameworkCitation {
        corpus: &'static str,
        citation_key: &'static str,
    },
 /// Differential-kernel agreement: every registered kernel in
 /// the `KernelRegistry` produced a unanimous verdict.
    DifferentialAgreement { kernels: Vec<&'static str> },
 /// Mutation-based property fuzzing produced unanimous agreement
 /// across registered kernels.
    DifferentialFuzz { iterations: usize },
 /// Cross-format roundtrip — one of the alternative backends
 /// (Coq / Lean / Agda / Dedukti / Isabelle) accepted the
 /// translated artifact.
    CrossFormat { backend: CrossFormatBackend },
 /// MSFS-corpus machine-verified theorem, with the corpus path
 /// carrying provenance.
    MsfsCorpus { corpus_path: &'static str },
 /// Admitted-with-IOU: the artifact is currently asserted via
 /// `@admit_reason(...)`. Audit gates must surface IOU count;
 /// not a full discharge.
    Iou { reason: IouReason },
 // ----------------------------------------------------------------
 // ATS-V foundation slots — placeholders per
 // `internal/specs/ats-v.md` §4 (architectural primitives).
 // These variants land empty in v0.1 of the verdict type and
 // are filled out by the ATS-V phase ( deliverable).
 // ----------------------------------------------------------------
 /// ATS-V capability flow check — discharged when a cog's
 /// declared `requires` list is satisfied by environment + no
 /// linear/affine capability is leaked.
    AtsVCapabilityCheck,
 /// ATS-V boundary type check — discharged when cross-module
 /// traffic conforms to the boundary's typed messages +
 /// invariants.
    AtsVBoundaryCheck,
 /// ATS-V composition correctness — discharged when `A ⊗ B`
 /// satisfies §5.3 composition rules.
    AtsVCompositionCheck,
 /// ATS-V anti-pattern absence — discharged when none of the
 /// 26+ canonical anti-patterns match the cog's `arch_type`.
    AtsVAntiPatternCheck { pattern_tag: &'static str },
 /// Multi-level meta-mode stability — discharged when the
 /// reflection-tower's constructive witness pattern is invariant
 /// across `[0, max_lift]` universe-ascent indices (MSFS Theorem
 /// 9.6(b) idempotence). Surfaces from
 /// [`crate::reflection_tower::walk_stability_up_to`].
    MetaModeStability { max_lift: u32 },
}

impl DischargeMethod {
 /// Stable diagnostic tag — short identifier used in audit JSON.
    pub fn tag(&self) -> &'static str {
        match self {
            DischargeMethod::Smt { .. } => "smt",
            DischargeMethod::KernelChecker { .. } => "kernel_checker",
            DischargeMethod::KernelIntrinsic { .. } => "kernel_intrinsic",
            DischargeMethod::FrameworkCitation { .. } => "framework_citation",
            DischargeMethod::DifferentialAgreement { .. } => "differential_agreement",
            DischargeMethod::DifferentialFuzz { .. } => "differential_fuzz",
            DischargeMethod::CrossFormat { .. } => "cross_format",
            DischargeMethod::MsfsCorpus { .. } => "msfs_corpus",
            DischargeMethod::Iou { .. } => "iou",
            DischargeMethod::AtsVCapabilityCheck => "ats_v_capability_check",
            DischargeMethod::AtsVBoundaryCheck => "ats_v_boundary_check",
            DischargeMethod::AtsVCompositionCheck => "ats_v_composition_check",
            DischargeMethod::AtsVAntiPatternCheck { .. } => "ats_v_anti_pattern_check",
            DischargeMethod::MetaModeStability { .. } => "meta_mode_stability",
        }
    }

 /// Is this method an ATS-V architectural-type check?
    pub fn is_ats_v(&self) -> bool {
        matches!(
            self,
            DischargeMethod::AtsVCapabilityCheck
                | DischargeMethod::AtsVBoundaryCheck
                | DischargeMethod::AtsVCompositionCheck
                | DischargeMethod::AtsVAntiPatternCheck { .. }
        )
    }
}

// =============================================================================
// Evidence / Counterexample / Pending — method-specific payloads
// =============================================================================

/// Method-specific witness payload for a `Discharged` verdict.
///
/// Kept structurally simple: a free-text `summary` plus structured
/// `metadata` map for downstream processing (audit JSON, LSP hover).
#[derive(Debug, Clone, Default)]
pub struct Evidence {
 /// Human-readable summary (single-line, audit-table friendly).
    pub summary: String,
 /// Structured metadata — JSON-encodable key/value pairs.
    pub metadata: BTreeMap<String, String>,
}

impl Evidence {
 /// Construct from a summary string with empty metadata.
    pub fn from_summary(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            metadata: BTreeMap::new(),
        }
    }

 /// Add a metadata entry, returning self for builder-style chains.
    pub fn with(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

/// Method-specific counterexample payload for a `Rejected` verdict.
#[derive(Debug, Clone, Default)]
pub struct Counterexample {
 /// Human-readable summary of the rejection.
    pub summary: String,
 /// Structured metadata (e.g. SMT model, failing kernel-rule
 /// position, mutation seed).
    pub metadata: BTreeMap<String, String>,
}

impl Counterexample {
    pub fn from_summary(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            metadata: BTreeMap::new(),
        }
    }

    pub fn with(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

/// Why a verdict landed in `Pending`. Distinct cases drive
/// distinct audit-report categorisation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingReason {
 /// SMT solver returned UNKNOWN (typically: timeout, fragment
 /// outside decidable theory, ground out of resources).
    SmtUnknown { detail: String },
 /// Kernel rejected with a "not yet implemented" / "stub" path.
    NotYetMechanised { detail: String },
 /// Differential-kernel slot not available (e.g., #154
 /// self-hosted Verum kernel pending parser fixes).
    NotYetSelfHosting,
 /// Generic timeout — method was given a bound and exhausted it.
    Timeout { milliseconds: u64 },
}

/// Structured IOU reason — admitted-with-citation discharge.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IouReason {
 /// `@framework(...)` citation to upstream literature.
    UpstreamCitation { corpus: &'static str },
 /// `@admit_reason("...")` with structured explanation.
    AdmittedWithReason { reason: String },
 /// Kernel-rule discharge route declared but not yet wired.
    PendingDispatch { intrinsic: &'static str },
}

// =============================================================================
// Auxiliary enums — backend tags
// =============================================================================

/// SMT backend identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SmtBackend {
    Z3,
    Cvc5,
    Portfolio,
}

/// Cross-format export backend identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CrossFormatBackend {
    Coq,
    Lean,
    Agda,
    Dedukti,
    Isabelle,
    Metamath,
}

// =============================================================================
// MultiVerdict — N-method aggregation
// =============================================================================

/// Aggregate of N method verdicts on the same artifact. Used
/// by the bundle-audit dispatcher and by the differential-kernel
/// registry.
#[derive(Debug, Clone)]
pub struct MultiVerdict {
 /// Per-method verdicts.
    pub verdicts: Vec<VerificationVerdict>,
}

impl MultiVerdict {
 /// Compute the aggregate verdict from per-method verdicts.
 /// Rules:
 /// * All `Discharged` → aggregate `Discharged` (with
 /// `DifferentialAgreement` method).
 /// * All `Rejected` → aggregate `Rejected` (with
 /// `DifferentialAgreement` method).
 /// * Mixed `Discharged` + `Rejected` → `Conflicted`.
 /// * Any `Pending` mixed in (without conflict) → aggregate
 /// pending; pending verdicts are propagated up.
    pub fn aggregate(&self) -> VerificationVerdict {
        let mut accepting: Vec<DischargeMethod> = Vec::new();
        let mut rejecting: Vec<DischargeMethod> = Vec::new();
        let mut pending: Vec<DischargeMethod> = Vec::new();

        for v in &self.verdicts {
            match v {
                VerificationVerdict::Discharged { method, .. } => {
                    accepting.push(method.clone())
                }
                VerificationVerdict::Rejected { method, .. } => rejecting.push(method.clone()),
                VerificationVerdict::Pending { method, .. } => pending.push(method.clone()),
                VerificationVerdict::Conflicted { .. } => {
 // A nested conflict propagates as conflict.
                    return VerificationVerdict::Conflicted {
                        accepting,
                        rejecting,
                    };
                }
            }
        }

        match (accepting.is_empty(), rejecting.is_empty()) {
            (false, true) => {
 // Unanimous accept. Pending methods are noted in
 // metadata but do not block the discharge.
                let kernel_names: Vec<&'static str> = accepting
                    .iter()
                    .filter_map(|m| match m {
                        DischargeMethod::KernelChecker { name } => Some(*name),
                        _ => None,
                    })
                    .collect();
                let summary = if !kernel_names.is_empty() {
                    format!(
                        "Unanimous accept across {} kernel(s): {}",
                        kernel_names.len(),
                        kernel_names.join(", "),
                    )
                } else {
                    format!(
                        "Unanimous accept across {} method(s)",
                        accepting.len(),
                    )
                };
                VerificationVerdict::Discharged {
                    method: DischargeMethod::DifferentialAgreement {
                        kernels: kernel_names,
                    },
                    evidence: Evidence::from_summary(summary)
                        .with("pending_methods", pending.len().to_string()),
                }
            }
            (true, false) => {
 // Unanimous reject.
                let kernel_names: Vec<&'static str> = rejecting
                    .iter()
                    .filter_map(|m| match m {
                        DischargeMethod::KernelChecker { name } => Some(*name),
                        _ => None,
                    })
                    .collect();
                VerificationVerdict::Rejected {
                    method: DischargeMethod::DifferentialAgreement {
                        kernels: kernel_names,
                    },
                    counterexample: Counterexample::from_summary(format!(
                        "Unanimous reject across {} method(s)",
                        rejecting.len(),
                    )),
                }
            }
            (false, false) => VerificationVerdict::Conflicted {
                accepting,
                rejecting,
            },
            (true, true) => {
 // No discharge happened (only pending). Surface
 // the first pending reason.
                let method = pending.first().cloned().unwrap_or(DischargeMethod::Iou {
                    reason: IouReason::AdmittedWithReason {
                        reason: "no method ran".into(),
                    },
                });
                VerificationVerdict::Pending {
                    method,
                    reason: PendingReason::NotYetMechanised {
                        detail: format!(
                            "{} pending method(s); zero discharges",
                            pending.len()
                        ),
                    },
                }
            }
        }
    }

 /// Number of verdicts that landed `Discharged`.
    pub fn discharged_count(&self) -> usize {
        self.verdicts.iter().filter(|v| v.is_discharged()).count()
    }

 /// Number of verdicts that landed `Rejected`.
    pub fn rejected_count(&self) -> usize {
        self.verdicts.iter().filter(|v| v.is_rejected()).count()
    }

 /// Number of verdicts that landed `Pending`.
    pub fn pending_count(&self) -> usize {
        self.verdicts.iter().filter(|v| v.is_pending()).count()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn discharged_kernel(name: &'static str) -> VerificationVerdict {
        VerificationVerdict::Discharged {
            method: DischargeMethod::KernelChecker { name },
            evidence: Evidence::from_summary("ok"),
        }
    }

    fn rejected_kernel(name: &'static str) -> VerificationVerdict {
        VerificationVerdict::Rejected {
            method: DischargeMethod::KernelChecker { name },
            counterexample: Counterexample::from_summary("bad"),
        }
    }

    fn pending_kernel(name: &'static str) -> VerificationVerdict {
        VerificationVerdict::Pending {
            method: DischargeMethod::KernelChecker { name },
            reason: PendingReason::NotYetMechanised {
                detail: "wip".into(),
            },
        }
    }

    #[test]
    fn verdict_tags_distinct() {
        let probes = [
            VerificationVerdict::Discharged {
                method: DischargeMethod::Smt {
                    backend: SmtBackend::Z3,
                },
                evidence: Evidence::default(),
            },
            VerificationVerdict::Rejected {
                method: DischargeMethod::Smt {
                    backend: SmtBackend::Z3,
                },
                counterexample: Counterexample::default(),
            },
            VerificationVerdict::Pending {
                method: DischargeMethod::Smt {
                    backend: SmtBackend::Z3,
                },
                reason: PendingReason::Timeout { milliseconds: 5000 },
            },
            VerificationVerdict::Conflicted {
                accepting: vec![],
                rejecting: vec![],
            },
        ];
        let tags: std::collections::BTreeSet<_> = probes.iter().map(|v| v.tag()).collect();
        assert_eq!(tags.len(), 4);
    }

    #[test]
    fn discharge_method_tags_distinct_includes_ats_v_slots() {
 // Pin: every variant has a distinct tag, including the
 // ATS-V foundation slots. This means future ATS-V
 // implementation can plug into the canonical verdict
 // without disturbing the existing audit-tag space.
        let probes = [
            DischargeMethod::Smt {
                backend: SmtBackend::Z3,
            },
            DischargeMethod::KernelChecker {
                name: "proof_checker",
            },
            DischargeMethod::KernelIntrinsic { name: "kernel_x" },
            DischargeMethod::FrameworkCitation {
                corpus: "msfs",
                citation_key: "thm_9_6",
            },
            DischargeMethod::DifferentialAgreement {
                kernels: vec!["a", "b"],
            },
            DischargeMethod::DifferentialFuzz { iterations: 100 },
            DischargeMethod::CrossFormat {
                backend: CrossFormatBackend::Coq,
            },
            DischargeMethod::MsfsCorpus {
                corpus_path: "theorems/msfs/...",
            },
            DischargeMethod::Iou {
                reason: IouReason::AdmittedWithReason {
                    reason: "wip".into(),
                },
            },
            DischargeMethod::AtsVCapabilityCheck,
            DischargeMethod::AtsVBoundaryCheck,
            DischargeMethod::AtsVCompositionCheck,
            DischargeMethod::AtsVAntiPatternCheck {
                pattern_tag: "capability_escalation",
            },
            DischargeMethod::MetaModeStability { max_lift: 10 },
        ];
        let tags: std::collections::BTreeSet<_> = probes.iter().map(|m| m.tag()).collect();
        assert_eq!(
            tags.len(),
            14,
            "every DischargeMethod variant must have a distinct tag",
        );
    }

    #[test]
    fn ats_v_methods_recognise_themselves() {
        assert!(DischargeMethod::AtsVCapabilityCheck.is_ats_v());
        assert!(DischargeMethod::AtsVBoundaryCheck.is_ats_v());
        assert!(DischargeMethod::AtsVCompositionCheck.is_ats_v());
        assert!(DischargeMethod::AtsVAntiPatternCheck {
            pattern_tag: "p"
        }
        .is_ats_v());
        assert!(!DischargeMethod::Smt {
            backend: SmtBackend::Z3
        }
        .is_ats_v());
        assert!(!DischargeMethod::KernelChecker { name: "x" }.is_ats_v());
    }

    #[test]
    fn aggregate_unanimous_accept() {
        let mv = MultiVerdict {
            verdicts: vec![
                discharged_kernel("proof_checker"),
                discharged_kernel("proof_checker_nbe"),
            ],
        };
        let agg = mv.aggregate();
        assert!(agg.is_discharged());
        match agg {
            VerificationVerdict::Discharged {
                method:
                    DischargeMethod::DifferentialAgreement {
                        kernels,
                    },
                ..
            } => {
                assert_eq!(kernels.len(), 2);
                assert!(kernels.contains(&"proof_checker"));
                assert!(kernels.contains(&"proof_checker_nbe"));
            }
            other => panic!("expected DifferentialAgreement Discharged, got {:?}", other),
        }
    }

    #[test]
    fn aggregate_unanimous_reject() {
        let mv = MultiVerdict {
            verdicts: vec![
                rejected_kernel("proof_checker"),
                rejected_kernel("proof_checker_nbe"),
            ],
        };
        let agg = mv.aggregate();
        assert!(agg.is_rejected());
    }

    #[test]
    fn aggregate_conflicted() {
        let mv = MultiVerdict {
            verdicts: vec![
                discharged_kernel("proof_checker"),
                rejected_kernel("synthetic_always_reject"),
            ],
        };
        let agg = mv.aggregate();
        assert!(agg.is_conflicted());
        match agg {
            VerificationVerdict::Conflicted {
                accepting,
                rejecting,
            } => {
                assert_eq!(accepting.len(), 1);
                assert_eq!(rejecting.len(), 1);
            }
            _ => panic!("expected Conflicted"),
        }
    }

    #[test]
    fn aggregate_pending_only() {
        let mv = MultiVerdict {
            verdicts: vec![pending_kernel("not_yet_mechanised")],
        };
        let agg = mv.aggregate();
        assert!(agg.is_pending());
    }

    #[test]
    fn aggregate_pending_does_not_mask_unanimous_accept() {
 // Pin: a pending verdict alongside unanimous accepts does
 // not block the discharge — pending is recorded as metadata
 // only.
        let mv = MultiVerdict {
            verdicts: vec![
                discharged_kernel("proof_checker"),
                discharged_kernel("proof_checker_nbe"),
                pending_kernel("verum_self_hosted_pending"),
            ],
        };
        assert!(mv.aggregate().is_discharged());
        assert_eq!(mv.discharged_count(), 2);
        assert_eq!(mv.pending_count(), 1);
    }

    #[test]
    fn evidence_builder_chains() {
        let e = Evidence::from_summary("ok")
            .with("kernel", "proof_checker")
            .with("rule", "K-Refine");
        assert_eq!(e.summary, "ok");
        assert_eq!(e.metadata.get("kernel"), Some(&"proof_checker".to_string()));
        assert_eq!(e.metadata.get("rule"), Some(&"K-Refine".to_string()));
    }

    #[test]
    fn primary_method_for_single_method_verdicts() {
        let d = discharged_kernel("proof_checker");
        match d.primary_method() {
            Some(DischargeMethod::KernelChecker {
                name: "proof_checker",
            }) => {}
            other => panic!("expected KernelChecker(proof_checker), got {:?}", other),
        }
        let c = VerificationVerdict::Conflicted {
            accepting: vec![],
            rejecting: vec![],
        };
        assert!(c.primary_method().is_none());
    }
}
