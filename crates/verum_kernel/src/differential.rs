//! Differential comparison of completed native proof verdicts.
//!
//! The substitution and NbE checkers are separate Rust algorithms. The optional
//! kernel_v0 slot wraps the substitution checker with bootstrap metadata checks;
//! it is not an independent self-hosted checker or a proof of kernel soundness.
//! Agreement on a finite corpus is regression evidence, not a soundness theorem.
//! Unsupported evidence and resource exhaustion cannot count as shared rejection.

#![warn(missing_docs)]

use crate::proof_checker::{self, Certificate, CheckError};
use crate::soundness::kernel_v0_manifest::{self, KernelV0Rule};
use serde::{Deserialize, Serialize};
use verum_proof::admission::{ReplayOutcome, classify_check_failure};

// =============================================================================
// KernelVerdict — one kernel's accept/reject answer for a certificate
// =============================================================================

/// One kernel's verdict on a [`Certificate`].
///
/// Both kernels are queried independently; their answers are
/// recorded here uniformly so the differential layer can compare
/// them without caring which kernel is which.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelVerdict {
    /// Checking did not establish a semantic verdict.
    Incomplete {
        /// Unsupported evidence, resource exhaustion, or another operational failure.
        outcome: ReplayOutcome,
        /// Original diagnostic retained without classifying it as rejection.
        reason: String,
    },
    /// The kernel accepted the certificate as a valid proof of its
    /// claimed type.
    Accepted,
    /// The kernel rejected the certificate. The `reason` is the
    /// kernel's own diagnostic surface — for the Rust side it's the
    /// `Display` of [`CheckError`]; for the Verum side it will be
    /// the equivalent formatted error once self-hosting lands.
    Rejected {
        /// Kernel-specific diagnostic message.
        reason: String,
    },
    /// The Verum-side kernel is not yet exercisable (parser blocker
    /// on `core/verify/kernel_v0/`). Used only by the Verum slot;
    /// the Rust slot never produces this verdict.
    NotYetSelfHosting,
}

impl KernelVerdict {
    /// Stable diagnostic tag — matches the serde representation.
    pub fn tag(&self) -> &'static str {
        match self {
            KernelVerdict::Incomplete { .. } => "incomplete",
            KernelVerdict::Accepted => "accepted",
            KernelVerdict::Rejected { .. } => "rejected",
            KernelVerdict::NotYetSelfHosting => "not_yet_self_hosting",
        }
    }

    /// Project: did this kernel accept the certificate?
    pub fn is_accepted(&self) -> bool {
        matches!(self, KernelVerdict::Accepted)
    }

    /// Project: did this kernel reject the certificate?
    pub fn is_rejected(&self) -> bool {
        matches!(self, KernelVerdict::Rejected { .. })
    }

    /// Project: is the Verum side stubbed-out for this verdict?
    pub fn is_not_yet_self_hosting(&self) -> bool {
        matches!(self, KernelVerdict::NotYetSelfHosting)
    }
}

impl From<Result<(), CheckError>> for KernelVerdict {
    /// Lift the Rust-side checker's `Result` into a [`KernelVerdict`].
    /// Semantic errors reject; operational/unsupported failures remain incomplete.
    /// Diagnostics are retained without determining the classification from prose.
    fn from(r: Result<(), CheckError>) -> Self {
        match r {
            Ok(()) => KernelVerdict::Accepted,
            Err(e) => {
                let (outcome, _) = classify_check_failure(&e);
                let reason = format!("{e:?}");
                if outcome == ReplayOutcome::Rejected {
                    Self::Rejected { reason }
                } else {
                    Self::Incomplete { outcome, reason }
                }
            }
        }
    }
}

/// Classify the independent checker's wire verdict without treating unknown or
/// operational errors as semantic rejection. A new protocol tag requires review.
pub fn classify_reference_verdict(accepted: bool, error: Option<&str>) -> ReplayOutcome {
    match (accepted, error) {
        (true, None) => ReplayOutcome::Accepted,
        (false, Some("FuelExhausted")) => ReplayOutcome::ResourceExhausted,
        (false, Some("UniverseOverflow" | "FoundationRequired" | "UnsupportedProfile")) => {
            ReplayOutcome::Unsupported
        }
        (
            false,
            Some(
                "InvalidInductive"
                | "UnboundVariable"
                | "NotAType"
                | "NotAFunction"
                | "NotASigma"
                | "NotAnIdentity"
                | "NotAValidJMotive"
                | "DomainMismatch"
                | "TypeMismatch"
                | "ClaimedTypeNotAType"
                | "ReflEndpointMismatch",
            ),
        ) => ReplayOutcome::Rejected,
        _ => ReplayOutcome::Inconclusive,
    }
}

// =============================================================================
// DifferentialAgreement — the inter-kernel verdict
// =============================================================================

/// The differential layer's verdict on a pair of [`KernelVerdict`]s.
///
/// `NotYetSelfHosting` is structurally distinct from `Disagreement`:
/// the former is "the test ran but only one kernel had a verdict to
/// offer" (architectural gap), the latter is "both kernels offered
/// verdicts and they disagreed" (audit failure).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DifferentialAgreement {
    /// At least one checker returned an operational failure, not a verdict.
    Incomplete,
    /// Both kernels accepted the certificate. The healthy path.
    BothAccept,
    /// Both kernels rejected the certificate. Also healthy — the
    /// test harness includes negative cases that *should* be
    /// rejected, and we want both kernels to reject in lock-step.
    BothReject,
    /// One kernel accepted, the other rejected. The audit-failure
    /// signal: the trusted bases have diverged on at least one
    /// certificate, and the audit gate must surface this loudly.
    Disagreement,
    /// The Verum side did not produce a verdict (parser blocker).
    /// Recorded separately so the audit gate can distinguish
    /// scaffolding gaps from real divergences.
    NotYetSelfHosting,
}

impl DifferentialAgreement {
    /// Stable diagnostic tag — matches the serde representation.
    pub fn tag(self) -> &'static str {
        match self {
            DifferentialAgreement::Incomplete => "incomplete",
            DifferentialAgreement::BothAccept => "both_accept",
            DifferentialAgreement::BothReject => "both_reject",
            DifferentialAgreement::Disagreement => "disagreement",
            DifferentialAgreement::NotYetSelfHosting => "not_yet_self_hosting",
        }
    }

    /// Classify two [`KernelVerdict`]s into the agreement category.
    ///
    /// `NotYetSelfHosting` on either side maps to
    /// `DifferentialAgreement::NotYetSelfHosting`; this preserves
    /// the architectural-gap signal even if the Rust side later
    /// learns to emit the same verdict for some reason.
    pub fn classify(rust: &KernelVerdict, verum: &KernelVerdict) -> Self {
        if rust.is_not_yet_self_hosting() || verum.is_not_yet_self_hosting() {
            return DifferentialAgreement::NotYetSelfHosting;
        }
        if matches!(rust, KernelVerdict::Incomplete { .. })
            || matches!(verum, KernelVerdict::Incomplete { .. })
        {
            return DifferentialAgreement::Incomplete;
        }
        match (rust.is_accepted(), verum.is_accepted()) {
            (true, true) => DifferentialAgreement::BothAccept,
            (false, false) => DifferentialAgreement::BothReject,
            _ => DifferentialAgreement::Disagreement,
        }
    }
}

// =============================================================================
// DifferentialReport — one row of the differential test result
// =============================================================================

/// One differential-test result: rule under test + both kernel
/// verdicts + the agreement classification.
///
/// Reports are designed for serialisation into
/// `target/audit-reports/differential.json` and consumption by
/// `verum audit --kernel-differential` (planned). Stable serde
/// representation is part of the contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DifferentialReport {
    /// The kernel rule under test (e.g. `"K-Var"`, `"K-Univ"`). Mirrors
    /// the `name` field of [`KernelV0Rule`].
    pub rule_name: String,
    /// The Rust-side kernel's verdict (always concrete: `Accepted`
    /// or `Rejected`).
    pub rust_verdict: KernelVerdict,
    /// The Verum-side kernel's verdict (currently always
    /// `NotYetSelfHosting` until the parser blocker lifts).
    pub verum_verdict: KernelVerdict,
    /// The classification combining both verdicts.
    pub agreement: DifferentialAgreement,
}

impl DifferentialReport {
    /// Construct a report from its components. Computes
    /// [`agreement`](Self::agreement) from the two verdicts so
    /// callers don't have to thread the classification manually.
    pub fn new(
        rule_name: impl Into<String>,
        rust_verdict: KernelVerdict,
        verum_verdict: KernelVerdict,
    ) -> Self {
        let agreement = DifferentialAgreement::classify(&rust_verdict, &verum_verdict);
        Self {
            rule_name: rule_name.into(),
            rust_verdict,
            verum_verdict,
            agreement,
        }
    }
}

// =============================================================================
// run_differential_test — the entry point
// =============================================================================

/// Run a differential test: invoke both kernels on `certificate` and
/// return a [`DifferentialReport`] describing the outcome.
///
/// Compares the native substitution and NbE algorithms from the registry.
/// The legacy `verum` field contains the NbE result; this function does not
/// invoke a self-hosted Verum checker.
pub fn run_differential_test(rule: &KernelV0Rule, certificate: &Certificate) -> DifferentialReport {
    // Canonical N-kernel discharge via the registry — single source
    // of truth for differential testing.  Outcomes are extracted in
    // registration order (proof_checker, proof_checker_nbe) and
    // mapped into the differential-report verdict shape.
    use crate::kernel_registry::KernelRegistry;
    let registry = KernelRegistry::default();
    let multi = registry.verify_all(certificate);
    let rust_verdict = multi
        .outcomes
        .iter()
        .find(|o| o.kernel_name == "proof_checker")
        .map(outcome_to_kernel_verdict)
        .unwrap_or(KernelVerdict::NotYetSelfHosting);
    let nbe_verdict = multi
        .outcomes
        .iter()
        .find(|o| o.kernel_name == "proof_checker_nbe")
        .map(outcome_to_kernel_verdict)
        .unwrap_or(KernelVerdict::NotYetSelfHosting);
    DifferentialReport::new(rule.name.clone(), rust_verdict, nbe_verdict)
}

fn outcome_to_kernel_verdict(outcome: &crate::kernel_registry::KernelOutcome) -> KernelVerdict {
    let reason = outcome.error_summary.clone().unwrap_or_default();
    match outcome.outcome {
        ReplayOutcome::Accepted => KernelVerdict::Accepted,
        ReplayOutcome::Rejected => KernelVerdict::Rejected { reason },
        other => KernelVerdict::Incomplete {
            outcome: other,
            reason,
        },
    }
}

/// Variant of [`run_differential_test`] that accepts a Verum-side
/// verdict directly. This is the **forward-compatible plug-in
/// point**: once `core/verify/kernel_v0/` is self-checking, the
/// caller obtains a [`KernelVerdict`] from it and passes it here.
///
/// Callers without a checker must pass [`KernelVerdict::NotYetSelfHosting`].
/// The standard [`run_differential_test`] instead compares two Rust algorithms.
pub fn run_differential_test_with_verum(
    rule: &KernelV0Rule,
    certificate: &Certificate,
    verum_verdict: KernelVerdict,
) -> DifferentialReport {
    let rust_verdict: KernelVerdict = certificate.verify().into();
    DifferentialReport::new(rule.name.clone(), rust_verdict, verum_verdict)
}

// =============================================================================
// Per-rule scaffolding
// =============================================================================

/// Run a stub differential test for the rule named `rule_name`.
///
/// Looks the rule up in [`kernel_v0_manifest::manifest`], builds a
/// trivial accept-path certificate (the polymorphic identity
/// `λ(A:U₀). λ(x:A). x : Π(A:U₀). A → A`), and runs it through
/// [`run_differential_test`]. Returns `None` if `rule_name` is not
/// in the manifest.
///
/// The certificate is the same one used by
/// `core/verify/proof_term_examples/polymorphic_identity.vproof` —
/// it covers T-Univ, T-Pi-Form, T-Lam-Intro, T-Var simultaneously,
/// so even though we only test against one rule's *manifest entry*
/// at a time, the actual verification exercises the full kernel.
///
/// This is sufficient scaffolding for the framework to be
/// load-bearing today; real per-rule certificates designed to
/// exercise *only* one rule (and reject under perturbation) land as
/// a follow-up once the Verum side comes online.
pub fn differential_test_rule(rule_name: &str) -> Option<DifferentialReport> {
    let rule = kernel_v0_manifest::manifest()
        .into_iter()
        .find(|r| r.name == rule_name)?;
    let certificate = stub_polymorphic_identity_certificate();
    Some(run_differential_test(&rule, &certificate))
}

/// Build the canonical polymorphic-identity certificate used by the
/// per-rule scaffolding. Closed term, accept-path under the Rust
/// kernel.
///
/// Term: `λ(A:Universe(0)). λ(x:Var(0)). Var(0)`
/// Type: `Π(A:Universe(0)). Π(x:Var(0)). Var(1)`
///
/// (The body's `Var(1)` refers to the outer-bound `A`; the inner
/// `Var(0)` is `x`; the outer Π's body has no `Var` reference to
/// the binder, but the inner Π does — this matches
/// `core/verify/proof_term_examples/polymorphic_identity.vproof`.)
pub fn stub_polymorphic_identity_certificate() -> Certificate {
    use proof_checker::Term;
    let term = Term::lam(Term::universe(0), Term::lam(Term::var(0), Term::var(0)));
    let claimed_type = Term::pi(Term::universe(0), Term::pi(Term::var(0), Term::var(1)));
    Certificate {
        term,
        claimed_type,
        metadata: std::collections::BTreeMap::new(),
    }
}

// =============================================================================
// DifferentialOutcome — aggregate over a batch of reports
// =============================================================================

/// Aggregate counts over a batch of [`DifferentialReport`]s.
///
/// Used by the audit gate to report the corpus-wide differential
/// status in a single JSON record:
/// `{ accepted: N, rejected: M, disagreement: K, not_yet_self_hosting: L }`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DifferentialOutcome {
    /// Reports where both kernels accepted.
    pub accepted: usize,
    /// Reports where both kernels rejected.
    pub rejected: usize,
    /// Reports where the kernels disagreed — the audit-failure count.
    pub disagreement: usize,
    /// Reports with operational failures rather than complete verdicts.
    pub incomplete: usize,
    /// Reports where the Verum side wasn't queryable.
    pub not_yet_self_hosting: usize,
}

impl DifferentialOutcome {
    /// Tally a slice of reports into a single outcome.
    pub fn from_reports(reports: &[DifferentialReport]) -> Self {
        let mut out = DifferentialOutcome::default();
        for r in reports {
            match r.agreement {
                DifferentialAgreement::Incomplete => out.incomplete += 1,
                DifferentialAgreement::BothAccept => out.accepted += 1,
                DifferentialAgreement::BothReject => out.rejected += 1,
                DifferentialAgreement::Disagreement => out.disagreement += 1,
                DifferentialAgreement::NotYetSelfHosting => out.not_yet_self_hosting += 1,
            }
        }
        out
    }

    /// Total report count across all categories.
    pub fn total(&self) -> usize {
        self.accepted
            + self.rejected
            + self.disagreement
            + self.incomplete
            + self.not_yet_self_hosting
    }

    /// A nonempty batch reached complete matching semantic verdicts.
    pub fn is_complete(&self) -> bool {
        self.total() > 0
            && self.incomplete == 0
            && self.not_yet_self_hosting == 0
            && self.disagreement == 0
    }

    /// Whether completed checker verdicts disagree. Incompleteness is separate.
    pub fn has_divergence(&self) -> bool {
        self.disagreement > 0
    }
}

// =============================================================================
// Tests — pin the report shape, agreement classifier, scaffolding
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operational_failure_is_never_both_reject() {
        let verdict: KernelVerdict = Err(CheckError::FuelExhausted).into();
        assert!(!verdict.is_rejected());
        let row = DifferentialReport::new("exhaustion", verdict.clone(), verdict);
        assert_eq!(row.agreement, DifferentialAgreement::Incomplete);
        let batch = DifferentialOutcome::from_reports(&[row]);
        assert_eq!(batch.rejected, 0);
        assert_eq!(batch.incomplete, 1);
        assert!(!batch.is_complete());
        assert!(!DifferentialOutcome::from_reports(&[]).is_complete());
    }

    #[test]
    fn wire_verdict_classification_never_guesses_rejection() {
        assert_eq!(
            classify_reference_verdict(true, None),
            ReplayOutcome::Accepted
        );
        assert_eq!(
            classify_reference_verdict(false, Some("TypeMismatch")),
            ReplayOutcome::Rejected
        );
        assert_eq!(
            classify_reference_verdict(false, Some("FuelExhausted")),
            ReplayOutcome::ResourceExhausted
        );
        assert_eq!(
            classify_reference_verdict(false, Some("FoundationRequired")),
            ReplayOutcome::Unsupported
        );
        for result in [
            (false, None),
            (false, Some("Unknown")),
            (true, Some("TypeMismatch")),
        ] {
            assert_eq!(
                classify_reference_verdict(result.0, result.1),
                ReplayOutcome::Inconclusive
            );
        }
    }

    /// Build a minimal `KernelV0Rule` for synthetic-path tests so we
    /// don't have to materialise the full manifest.
    fn synthetic_rule(name: &str) -> KernelV0Rule {
        use crate::soundness::DischargeStatus;
        KernelV0Rule {
            name: name.to_string(),
            lemma_symbol: format!("k_{}_sound", name.to_ascii_lowercase()),
            file_path: std::path::PathBuf::from("verify/kernel_v0/rules/k_synth.vr"),
            status: DischargeStatus::Discharged,
            description: "synthetic test rule".to_string(),
        }
    }

    #[test]
    fn report_shape_carries_all_four_fields() {
        // Pin the public surface: a report exposes rule_name,
        // rust_verdict, verum_verdict, agreement. Drift here breaks
        // the audit-gate JSON contract.
        //
        // **Post-V1 (#159)**: the second slot now runs the NbE
        // kernel (`proof_checker_nbe`). On the canonical poly-id
        // certificate both kernels accept → `BothAccept`.
        let rule = synthetic_rule("K-Synth");
        let cert = stub_polymorphic_identity_certificate();
        let report = run_differential_test(&rule, &cert);
        assert_eq!(report.rule_name, "K-Synth");
        assert!(report.rust_verdict.is_accepted());
        assert!(report.verum_verdict.is_accepted());
        assert_eq!(report.agreement, DifferentialAgreement::BothAccept);
    }

    #[test]
    fn both_accept_path_classifies_correctly() {
        // Synthetic: both kernels accept. This is the future
        // healthy-path classification once the Verum side comes
        // online.
        let rust = KernelVerdict::Accepted;
        let verum = KernelVerdict::Accepted;
        assert_eq!(
            DifferentialAgreement::classify(&rust, &verum),
            DifferentialAgreement::BothAccept,
        );
    }

    #[test]
    fn both_reject_path_classifies_correctly() {
        // Synthetic: both kernels reject. Negative-case healthy
        // path — the harness includes invalid certificates that
        // *should* be rejected, and we want lock-step rejection.
        let rust = KernelVerdict::Rejected {
            reason: "UnboundVariable(0)".to_string(),
        };
        let verum = KernelVerdict::Rejected {
            reason: "k_var: index 0 out of bounds".to_string(),
        };
        assert_eq!(
            DifferentialAgreement::classify(&rust, &verum),
            DifferentialAgreement::BothReject,
        );
    }

    #[test]
    fn disagreement_path_classifies_correctly() {
        // Synthetic: Rust accepts, Verum rejects. The audit-fail
        // signal — at least one kernel has a soundness or
        // completeness bug.
        let rust = KernelVerdict::Accepted;
        let verum = KernelVerdict::Rejected {
            reason: "synthetic divergence".to_string(),
        };
        assert_eq!(
            DifferentialAgreement::classify(&rust, &verum),
            DifferentialAgreement::Disagreement,
        );

        // And the symmetric direction (Rust rejects, Verum accepts)
        // — equally a divergence.
        let rust = KernelVerdict::Rejected {
            reason: "synthetic divergence".to_string(),
        };
        let verum = KernelVerdict::Accepted;
        assert_eq!(
            DifferentialAgreement::classify(&rust, &verum),
            DifferentialAgreement::Disagreement,
        );
    }

    #[test]
    fn not_yet_self_hosting_path_classifies_correctly() {
        // Pin the architectural-gap signal. Either side reporting
        // NotYetSelfHosting collapses the verdict to
        // NotYetSelfHosting, even if the other side has a concrete
        // accept/reject answer. This is what keeps the framework
        // load-bearing today: every Verum-side query is
        // NotYetSelfHosting, and the harness records that distinctly
        // from real disagreement.
        let rust = KernelVerdict::Accepted;
        let verum = KernelVerdict::NotYetSelfHosting;
        assert_eq!(
            DifferentialAgreement::classify(&rust, &verum),
            DifferentialAgreement::NotYetSelfHosting,
        );

        let rust = KernelVerdict::Rejected {
            reason: "anything".to_string(),
        };
        let verum = KernelVerdict::NotYetSelfHosting;
        assert_eq!(
            DifferentialAgreement::classify(&rust, &verum),
            DifferentialAgreement::NotYetSelfHosting,
        );
    }

    #[test]
    fn kernel_v0_rule_lemma_symbol_is_consumable() {
        // Contract test: the manifest's `lemma_symbol` field is the
        // hand-off point to the Verum-side checker. Verify the
        // first manifest rule's lemma_symbol obeys the
        // `k_<name>_sound` convention so the harness's future Verum-
        // side adapter can reach it by deterministic naming.
        let rule = kernel_v0_manifest::manifest()
            .into_iter()
            .next()
            .expect("manifest has at least one rule");
        assert!(
            rule.lemma_symbol.starts_with("k_"),
            "lemma_symbol {:?} should start with `k_`",
            rule.lemma_symbol,
        );
        assert!(
            rule.lemma_symbol.ends_with("_sound"),
            "lemma_symbol {:?} should end with `_sound`",
            rule.lemma_symbol,
        );
        // And a stub differential-test against it executes cleanly.
        let report = differential_test_rule(&rule.name).expect("manifest rule resolvable by name");
        assert_eq!(report.rule_name, rule.name);
    }

    #[test]
    fn differential_test_rule_returns_none_for_unknown_name() {
        // Defensive: the per-rule scaffolding rejects unknown rule
        // names rather than panicking, since callers may iterate
        // over a heterogeneous source of rule names.
        assert!(differential_test_rule("K-Does-Not-Exist").is_none());
    }

    #[test]
    fn differential_outcome_from_reports_tallies_correctly() {
        let reports = vec![
            DifferentialReport::new("r1", KernelVerdict::Accepted, KernelVerdict::Accepted),
            DifferentialReport::new(
                "r2",
                KernelVerdict::Rejected {
                    reason: "x".to_string(),
                },
                KernelVerdict::Rejected {
                    reason: "y".to_string(),
                },
            ),
            DifferentialReport::new(
                "r3",
                KernelVerdict::Accepted,
                KernelVerdict::Rejected {
                    reason: "divergence".to_string(),
                },
            ),
            DifferentialReport::new(
                "r4",
                KernelVerdict::Accepted,
                KernelVerdict::NotYetSelfHosting,
            ),
        ];
        let outcome = DifferentialOutcome::from_reports(&reports);
        assert_eq!(outcome.accepted, 1);
        assert_eq!(outcome.rejected, 1);
        assert_eq!(outcome.disagreement, 1);
        assert_eq!(outcome.not_yet_self_hosting, 1);
        assert_eq!(outcome.total(), 4);
        assert!(outcome.has_divergence());
    }

    #[test]
    fn differential_outcome_no_divergence_with_active_nbe_second_slot() {
        // **Post-V1 (#159)**: the second slot is the NbE kernel.
        // On the canonical polymorphic-identity certificate both
        // kernels accept; the audit gate reports BothAccept for
        // every rule.  Pre-V1 this test asserted
        // NotYetSelfHosting, since the second slot was stubbed.
        let reports: Vec<_> = kernel_v0_manifest::manifest()
            .iter()
            .map(|rule| run_differential_test(rule, &stub_polymorphic_identity_certificate()))
            .collect();
        let outcome = DifferentialOutcome::from_reports(&reports);
        assert_eq!(outcome.disagreement, 0);
        assert_eq!(outcome.not_yet_self_hosting, 0);
        assert_eq!(outcome.accepted, kernel_v0_manifest::KERNEL_V0_RULE_COUNT);
        assert!(!outcome.has_divergence());
    }

    #[test]
    fn run_differential_test_with_verum_threads_explicit_verdict() {
        // Pin the forward-compatible plug-in point: when the Verum
        // side comes online, the caller passes its verdict in via
        // run_differential_test_with_verum() and the agreement
        // classifier handles the rest.
        let rule = synthetic_rule("K-PlugIn");
        let cert = stub_polymorphic_identity_certificate();
        let report = run_differential_test_with_verum(&rule, &cert, KernelVerdict::Accepted);
        assert_eq!(report.agreement, DifferentialAgreement::BothAccept);

        let report = run_differential_test_with_verum(
            &rule,
            &cert,
            KernelVerdict::Rejected {
                reason: "synthetic Verum-side rejection".to_string(),
            },
        );
        assert_eq!(report.agreement, DifferentialAgreement::Disagreement);
    }

    #[test]
    fn report_serde_round_trip() {
        let report = DifferentialReport::new(
            "K-Var",
            KernelVerdict::Accepted,
            KernelVerdict::NotYetSelfHosting,
        );
        let json = serde_json::to_string(&report).expect("serialise");
        let restored: DifferentialReport = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(restored, report);
    }

    #[test]
    fn outcome_serde_round_trip() {
        let outcome = DifferentialOutcome {
            accepted: 7,
            rejected: 3,
            disagreement: 0,
            incomplete: 0,
            not_yet_self_hosting: 10,
        };
        let json = serde_json::to_string(&outcome).expect("serialise");
        let restored: DifferentialOutcome = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(restored, outcome);
    }

    #[test]
    fn verdict_tags_are_stable() {
        // The serde tags are part of the audit-gate JSON contract.
        // Pin them so a rename of a variant breaks this test loudly.
        assert_eq!(KernelVerdict::Accepted.tag(), "accepted");
        assert_eq!(
            KernelVerdict::Rejected {
                reason: String::new()
            }
            .tag(),
            "rejected",
        );
        assert_eq!(
            KernelVerdict::NotYetSelfHosting.tag(),
            "not_yet_self_hosting",
        );

        assert_eq!(DifferentialAgreement::BothAccept.tag(), "both_accept");
        assert_eq!(DifferentialAgreement::BothReject.tag(), "both_reject");
        assert_eq!(DifferentialAgreement::Disagreement.tag(), "disagreement",);
        assert_eq!(
            DifferentialAgreement::NotYetSelfHosting.tag(),
            "not_yet_self_hosting",
        );
    }

    #[test]
    fn verdict_from_check_result_lifts_correctly() {
        let ok: KernelVerdict = Result::<(), CheckError>::Ok(()).into();
        assert!(ok.is_accepted());

        let err: KernelVerdict =
            Result::<(), CheckError>::Err(CheckError::UnboundVariable(0)).into();
        assert!(err.is_rejected());
        if let KernelVerdict::Rejected { reason } = err {
            assert!(
                reason.contains("UnboundVariable"),
                "reason should include the kernel error variant, got {:?}",
                reason,
            );
        } else {
            panic!("expected Rejected");
        }
    }
}
