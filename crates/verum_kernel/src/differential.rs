//! Differential-kernel testing harness (#159).
//!

//! Verum runs **two** kernel implementations in parallel:
//!

//! 1. The Rust trusted base — [`crate::proof_checker`] — 633 LOC of
//!  bidirectional type-checking over a minimal CoC fragment, the
//!  answer reviewers receive when they ask "what do I need to
//!  trust to trust Verum?".
//!

//! 2. The Verum-side scaffold — `core/verify/kernel_v0/` — a Verum-
//!  self-hosted mirror of the same 10-rule bootstrap kernel, whose
//!  soundness lemmas are enumerated by
//!  [`crate::soundness::kernel_v0_manifest`].
//!

//! Differential testing checks that **both** implementations agree on
//! every test certificate: either they both accept (`Both_Accept`),
//! both reject (`Both_Reject`), or they disagree (`Disagreement`) —
//! the last case being the audit failure mode that surfaces a kernel
//! divergence before it reaches the trust boundary.
//!

//! ## Current status — TWO Rust-side kernels active (#159 V1)
//!

//! Pre-V1 the second slot was stubbed as `NotYetSelfHosting`.
//! Post-V1 the second slot runs [`crate::proof_checker_nbe`] —
//! a structurally-distinct algorithmic kernel using
//! Normalisation-by-Evaluation.  The two implementations
//! (bidirectional + explicit substitution vs NbE) compute the
//! same input/output relation via different evaluation strategies;
//! disagreements are bugs in EITHER side.
//!

//! The Verum-self-hosted kernel (`core/verify/kernel_v0/`) is
//! tracked separately under #154 and will land as a THIRD slot
//! once the parser blocker lands.  Until then `proof_checker_nbe`
//! is the active second kernel.
//!

//! This module is **load-bearing scaffolding**: the entire framework
//! is in place — `DifferentialReport`, `run_differential_test`, the
//! per-rule scaffold [`differential_test_rule`], the
//! [`DifferentialOutcome`] aggregator — so that plugging in a real
//! Verum-side checker is a **single-line addition** to
//! [`run_differential_test_with_verum`]. The audit gate, the JSON
//! emitter, the test harness, and the per-rule pin tests all consume
//! this surface today and will continue to consume it unchanged once
//! the Verum side comes online.
//!

//! ## What this module does NOT do
//!

//! - Does NOT fix the parser blocker on `core/verify/kernel_v0/`.
//!  That's tracked separately as a multi-day Verum-compiler effort.
//! - Does NOT shell out to a Verum binary. When the Verum side
//!  becomes self-checking the integration is via a Rust trait
//!  implementation, not via process invocation.
//! - Does NOT promote disagreement into a panic. The harness
//!  *records* divergence; the audit gate *interprets* it.

#![warn(missing_docs)]

use serde::{Deserialize, Serialize};

use crate::proof_checker::{self, Certificate, CheckError};
use crate::soundness::kernel_v0_manifest::{self, KernelV0Rule};

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
    /// `Ok(())` → `Accepted`; `Err(e)` → `Rejected { reason: <Debug>>`.
    /// The `Debug` projection is stable for the kernel's error
    /// surface (no float/pointer values reach `CheckError`).
    fn from(r: Result<(), CheckError>) -> Self {
        match r {
            Ok(()) => KernelVerdict::Accepted,
            Err(e) => KernelVerdict::Rejected {
                reason: format!("{:?}", e),
            },
        }
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

/// The Rust side is queried via [`Certificate::verify`]. The Verum
/// side is currently stubbed as [`KernelVerdict::NotYetSelfHosting`]
/// pending the parser blocker on `core/verify/kernel_v0/`; once the
/// Verum-side checker is invokable, plug it in via
/// [`run_differential_test_with_verum`] and the agreement
/// classification will become live without touching this function's
/// callers.
pub fn run_differential_test(rule: &KernelV0Rule, certificate: &Certificate) -> DifferentialReport {
    let rust_verdict: KernelVerdict = certificate.verify().into();
    // **#159 V1 — second algorithmic kernel**.  Pre-V1 the second
    // slot was stubbed as `NotYetSelfHosting` because no second
    // implementation existed.  Post-V1 the NbE-based
    // [`crate::proof_checker_nbe`] runs as the second kernel.
    // Differential-test catches implementation bugs in EITHER
    // implementation as Disagreement verdicts.
    //
    // The Verum-self-hosted kernel (`core/verify/kernel_v0/`) is
    // a separate, longer-running project tracked under #154; when
    // it lands it will become a THIRD slot via a separate
    // `run_differential_test_with_verum` invocation.
    let nbe_verdict: KernelVerdict =
        crate::proof_checker_nbe::verify_certificate(certificate).into();
    DifferentialReport::new(rule.name.clone(), rust_verdict, nbe_verdict)
}

/// Variant of [`run_differential_test`] that accepts a Verum-side
/// verdict directly. This is the **forward-compatible plug-in
/// point**: once `core/verify/kernel_v0/` is self-checking, the
/// caller obtains a [`KernelVerdict`] from it and passes it here.
///

/// Exposed publicly so the future Verum-side adapter can route
/// through it without touching this module. Until then, callers
/// pass [`KernelVerdict::NotYetSelfHosting`] explicitly when they
/// want the future-proof shape (the standard
/// [`run_differential_test`] supplies that default).
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
    /// Reports where the Verum side wasn't queryable.
    pub not_yet_self_hosting: usize,
}

impl DifferentialOutcome {
    /// Tally a slice of reports into a single outcome.
    pub fn from_reports(reports: &[DifferentialReport]) -> Self {
        let mut out = DifferentialOutcome::default();
        for r in reports {
            match r.agreement {
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
        self.accepted + self.rejected + self.disagreement + self.not_yet_self_hosting
    }

    /// Project: are there any divergences? This is the audit-fail
    /// predicate the gate consumes. `NotYetSelfHosting` is *not* a
    /// divergence (the Verum side simply isn't online); only
    /// `Disagreement` flips this to true.
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

    /// Build a minimal `KernelV0Rule` for synthetic-path tests so we
    /// don't have to materialise the full manifest.
    fn synthetic_rule(name: &str) -> KernelV0Rule {
        use kernel_v0_manifest::KernelV0Status;
        KernelV0Rule {
            name: name.to_string(),
            lemma_symbol: format!("k_{}_sound", name.to_ascii_lowercase()),
            file_path: std::path::PathBuf::from("verify/kernel_v0/rules/k_synth.vr"),
            status: KernelV0Status::Proved,
            description: "synthetic test rule".to_string(),
            iou_citation: String::new(),
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
