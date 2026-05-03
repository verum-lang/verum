//! N-kernel registry for differential testing (#159 V4).
//!
//! ## Architectural role
//!
//! Pre-this-module, `differential::run_differential_test` was
//! hardcoded to TWO kernel slots:
//!   * Slot 1: `proof_checker::Certificate::verify` (trusted base).
//!   * Slot 2: `proof_checker_nbe::verify_certificate` (NbE, V1).
//!
//! Adding a third implementation (#154 self-hosted Verum kernel,
//! or future algorithmic alternatives like HOAS-based checking)
//! would require touching the differential dispatcher every time.
//!
//! This module ships the **N-kernel registry pattern**:
//!   * [`KernelChecker`] trait — abstract kernel interface.
//!   * [`KernelRegistry`] — collection of registered kernels.
//!   * [`run_differential_n`] — runs a certificate through every
//!     registered kernel and reports per-pair agreement.
//!   * [`MultiVerdict`] — N-tuple of per-kernel verdicts with
//!     pairwise-agreement classification.
//!
//! Adding a new kernel implementation is now ONE line:
//! ```ignore
//! registry.register(MyKernelImpl);
//! ```
//! No changes to the differential dispatcher, the audit gate, or
//! the JSON report shape — they all walk the registry uniformly.
//!
//! ## Soundness
//!
//! All registered kernels MUST agree on every certificate
//! (lock-step accept-or-reject). The N-kernel agreement classifier
//! reports `Unanimous` (all agree on accept), `UnanimousReject`
//! (all agree on reject), or `Disagreement(pairs)` enumerating
//! which kernels disagree. The audit gate flips to failure on
//! ANY disagreement.

use crate::proof_checker::{Certificate, CheckError};
use crate::proof_checker_nbe;

// =============================================================================
// KernelChecker trait — the abstraction
// =============================================================================

/// Abstract kernel interface. Implementations register with a
/// [`KernelRegistry`] to participate in differential testing.
///
/// **Invariant**: a kernel's verdict must be deterministic — the
/// same certificate, fed twice, must produce the same verdict.
/// Non-determinism would make differential testing meaningless.
pub trait KernelChecker: Send + Sync {
    /// Stable kernel-implementation identifier. Used in audit
    /// reports + agreement classification.
    fn name(&self) -> &'static str;

    /// One-line description of the algorithm + intended trust role.
    fn description(&self) -> &'static str;

    /// Verify a certificate.  Returns `Ok(())` for accept,
    /// `Err(CheckError)` for reject.
    fn verify(&self, cert: &Certificate) -> Result<(), CheckError>;
}

// =============================================================================
// Built-in kernel implementations
// =============================================================================

/// Algorithm A — the trusted base.  Bidirectional type-checking
/// with explicit substitution + WHNF normalisation. The 633-LOC
/// `proof_checker.rs` minimal kernel.
pub struct ProofCheckerKernel;

impl KernelChecker for ProofCheckerKernel {
    fn name(&self) -> &'static str {
        "proof_checker"
    }

    fn description(&self) -> &'static str {
        "Algorithm A — Bidirectional type-checking with explicit substitution + WHNF \
         (the 633-LOC trusted base, `proof_checker.rs`)"
    }

    fn verify(&self, cert: &Certificate) -> Result<(), CheckError> {
        cert.verify()
    }
}

/// Algorithm B — Normalisation by Evaluation.  Closure-based
/// semantic evaluation + level-indexed quote.  Structurally
/// distinct from Algorithm A; differential-tested against it.
pub struct ProofCheckerNbeKernel;

impl KernelChecker for ProofCheckerNbeKernel {
    fn name(&self) -> &'static str {
        "proof_checker_nbe"
    }

    fn description(&self) -> &'static str {
        "Algorithm B — Normalisation by Evaluation with closures + level-indexed quote \
         (the 756-LOC second kernel, `proof_checker_nbe.rs`)"
    }

    fn verify(&self, cert: &Certificate) -> Result<(), CheckError> {
        proof_checker_nbe::verify_certificate(cert)
    }
}

// =============================================================================
// KernelRegistry — the N-kernel collection
// =============================================================================

/// Collection of registered kernel implementations.
///
/// **Default**: builds a registry with the two algorithmically-
/// distinct production kernels (`ProofCheckerKernel` +
/// `ProofCheckerNbeKernel`).  Callers can construct an empty
/// registry via `KernelRegistry::new()` and register custom
/// implementations.
pub struct KernelRegistry {
    kernels: Vec<Box<dyn KernelChecker>>,
}

impl KernelRegistry {
    /// Construct an empty registry.
    pub fn new() -> Self {
        Self { kernels: Vec::new() }
    }

    /// Register a kernel implementation.
    pub fn register(&mut self, kernel: impl KernelChecker + 'static) {
        self.kernels.push(Box::new(kernel));
    }

    /// Number of registered kernels.
    pub fn len(&self) -> usize {
        self.kernels.len()
    }

    /// True iff no kernels are registered.
    pub fn is_empty(&self) -> bool {
        self.kernels.is_empty()
    }

    /// Iterate the registered kernels' names.  Stable order:
    /// registration order.
    pub fn names(&self) -> Vec<&'static str> {
        self.kernels.iter().map(|k| k.name()).collect()
    }

    /// Verify a certificate through every registered kernel.
    /// Returns a `MultiVerdict` carrying per-kernel verdicts +
    /// pairwise-agreement classification.
    pub fn verify_all(&self, cert: &Certificate) -> MultiVerdict {
        let mut verdicts: Vec<KernelOutcome> = Vec::with_capacity(self.kernels.len());
        for kernel in &self.kernels {
            let result = kernel.verify(cert);
            verdicts.push(KernelOutcome {
                kernel_name: kernel.name(),
                accepted: result.is_ok(),
                error_summary: result.err().map(|e| format!("{:?}", e)),
            });
        }
        MultiVerdict::from_outcomes(verdicts)
    }
}

impl Default for KernelRegistry {
    /// Default registry: the two production-ready kernels.
    /// Future kernel implementations register on top of this.
    fn default() -> Self {
        let mut r = Self::new();
        r.register(ProofCheckerKernel);
        r.register(ProofCheckerNbeKernel);
        r
    }
}

// =============================================================================
// MultiVerdict — N-kernel agreement
// =============================================================================

/// One kernel's outcome on a certificate.
#[derive(Debug, Clone)]
pub struct KernelOutcome {
    /// Name of the kernel implementation that produced this verdict.
    pub kernel_name: &'static str,
    /// `true` iff the kernel accepted.
    pub accepted: bool,
    /// `Some(_)` iff the kernel rejected — formatted error.
    pub error_summary: Option<String>,
}

/// N-kernel agreement classification.
///
/// `Unanimous` (all accept) and `UnanimousReject` (all reject) are
/// the soundness-clean outcomes. `Disagreement` lists which
/// kernels disagreed — used by the audit gate to surface the
/// specific divergence.
#[derive(Debug, Clone)]
pub enum AgreementVerdict {
    /// All registered kernels accepted.
    Unanimous,
    /// All registered kernels rejected.
    UnanimousReject,
    /// Kernels disagreed.  `accepting` lists the names of kernels
    /// that accepted; `rejecting` lists those that rejected.  Both
    /// non-empty.
    Disagreement {
        /// Kernel names that accepted.
        accepting: Vec<&'static str>,
        /// Kernel names that rejected.
        rejecting: Vec<&'static str>,
    },
}

impl AgreementVerdict {
    /// Stable diagnostic tag for audit reports.
    pub fn tag(&self) -> &'static str {
        match self {
            AgreementVerdict::Unanimous => "unanimous_accept",
            AgreementVerdict::UnanimousReject => "unanimous_reject",
            AgreementVerdict::Disagreement { .. } => "disagreement",
        }
    }

    /// True iff all kernels agreed (any direction). Disagreements
    /// are the load-bearing failure mode.
    pub fn is_unanimous(&self) -> bool {
        !matches!(self, AgreementVerdict::Disagreement { .. })
    }
}

/// N-kernel verdict on a single certificate. Carries every
/// per-kernel outcome plus the unified agreement classifier.
#[derive(Debug, Clone)]
pub struct MultiVerdict {
    /// Per-kernel outcomes, in registration order.
    pub outcomes: Vec<KernelOutcome>,
    /// Pairwise-agreement classification.
    pub agreement: AgreementVerdict,
}

impl MultiVerdict {
    /// Build from per-kernel outcomes. Computes the agreement
    /// classifier from the outcome set.
    pub fn from_outcomes(outcomes: Vec<KernelOutcome>) -> Self {
        let accepting: Vec<&'static str> = outcomes
            .iter()
            .filter(|o| o.accepted)
            .map(|o| o.kernel_name)
            .collect();
        let rejecting: Vec<&'static str> = outcomes
            .iter()
            .filter(|o| !o.accepted)
            .map(|o| o.kernel_name)
            .collect();
        let agreement = match (accepting.is_empty(), rejecting.is_empty()) {
            (false, true) => AgreementVerdict::Unanimous,
            (true, false) => AgreementVerdict::UnanimousReject,
            (false, false) => AgreementVerdict::Disagreement {
                accepting,
                rejecting,
            },
            // (true, true) is impossible with non-empty outcomes;
            // if outcomes is empty, treat as unanimous (degenerate
            // — no kernels to disagree).
            (true, true) => AgreementVerdict::Unanimous,
        };
        Self {
            outcomes,
            agreement,
        }
    }
}

// =============================================================================
// Top-level API
// =============================================================================

/// Run a certificate through every kernel in the default registry
/// and return the multi-verdict.  Convenience wrapper for the
/// common case.
pub fn run_differential_n(cert: &Certificate) -> MultiVerdict {
    KernelRegistry::default().verify_all(cert)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proof_checker::Term;

    fn polymorphic_identity() -> Certificate {
        let term = Term::lam(
            Term::universe(0),
            Term::lam(Term::var(0), Term::var(0)),
        );
        let claimed_type = Term::pi(
            Term::universe(0),
            Term::pi(Term::var(0), Term::var(1)),
        );
        Certificate {
            term,
            claimed_type,
            metadata: std::collections::BTreeMap::new(),
        }
    }

    fn invalid_cert() -> Certificate {
        // Universe(0) : Universe(0) — should reject.
        Certificate {
            term: Term::Universe(0),
            claimed_type: Term::Universe(0),
            metadata: std::collections::BTreeMap::new(),
        }
    }

    // ----- Default registry -----

    #[test]
    fn default_registry_has_two_kernels() {
        let r = KernelRegistry::default();
        assert_eq!(r.len(), 2);
        assert!(!r.is_empty());
        let names = r.names();
        assert!(names.contains(&"proof_checker"));
        assert!(names.contains(&"proof_checker_nbe"));
    }

    #[test]
    fn empty_registry_is_empty() {
        let r = KernelRegistry::new();
        assert_eq!(r.len(), 0);
        assert!(r.is_empty());
    }

    // ----- Multi-kernel verdict on accept certs -----

    #[test]
    fn polymorphic_identity_is_unanimous_accept() {
        let r = KernelRegistry::default();
        let v = r.verify_all(&polymorphic_identity());
        assert!(matches!(v.agreement, AgreementVerdict::Unanimous));
        assert!(v.agreement.is_unanimous());
        assert_eq!(v.outcomes.len(), 2);
        for o in &v.outcomes {
            assert!(o.accepted);
            assert!(o.error_summary.is_none());
        }
    }

    #[test]
    fn invalid_cert_is_unanimous_reject() {
        let r = KernelRegistry::default();
        let v = r.verify_all(&invalid_cert());
        assert!(matches!(v.agreement, AgreementVerdict::UnanimousReject));
        assert!(v.agreement.is_unanimous());
        for o in &v.outcomes {
            assert!(!o.accepted);
            assert!(o.error_summary.is_some());
        }
    }

    // ----- Custom kernel registration -----

    /// Synthetic kernel that always accepts — used to engineer a
    /// disagreement against the trusted base on an invalid cert.
    struct AlwaysAcceptKernel;
    impl KernelChecker for AlwaysAcceptKernel {
        fn name(&self) -> &'static str {
            "always_accept_synthetic"
        }
        fn description(&self) -> &'static str {
            "synthetic — always accepts (test-only)"
        }
        fn verify(&self, _cert: &Certificate) -> Result<(), CheckError> {
            Ok(())
        }
    }

    #[test]
    fn registry_supports_custom_kernel_registration() {
        let mut r = KernelRegistry::new();
        r.register(ProofCheckerKernel);
        r.register(AlwaysAcceptKernel);
        assert_eq!(r.len(), 2);
        // On an invalid cert, the trusted base rejects but
        // AlwaysAcceptKernel accepts → disagreement.
        let v = r.verify_all(&invalid_cert());
        match v.agreement {
            AgreementVerdict::Disagreement {
                accepting,
                rejecting,
            } => {
                assert!(accepting.contains(&"always_accept_synthetic"));
                assert!(rejecting.contains(&"proof_checker"));
            }
            other => panic!("expected Disagreement, got {:?}", other),
        }
    }

    #[test]
    fn agreement_verdict_tags_are_distinct() {
        // Pin: every variant has a distinct stable tag.
        let probes = [
            AgreementVerdict::Unanimous,
            AgreementVerdict::UnanimousReject,
            AgreementVerdict::Disagreement {
                accepting: vec!["a"],
                rejecting: vec!["b"],
            },
        ];
        let tags: std::collections::BTreeSet<_> = probes.iter().map(|v| v.tag()).collect();
        assert_eq!(tags.len(), 3);
    }

    #[test]
    fn run_differential_n_convenience_wraps_default_registry() {
        let v = run_differential_n(&polymorphic_identity());
        assert!(v.agreement.is_unanimous());
        assert_eq!(v.outcomes.len(), 2);
    }

    // ----- Architectural pin -----

    #[test]
    fn registration_order_is_preserved() {
        // Pin: outcomes appear in registration order, not arbitrary.
        let mut r = KernelRegistry::new();
        r.register(ProofCheckerNbeKernel); // NbE first
        r.register(ProofCheckerKernel); // trusted base second
        let v = r.verify_all(&polymorphic_identity());
        assert_eq!(v.outcomes[0].kernel_name, "proof_checker_nbe");
        assert_eq!(v.outcomes[1].kernel_name, "proof_checker");
    }

    #[test]
    fn three_kernel_unanimous_when_all_agree() {
        // Adding a third kernel that mirrors trusted-base behaviour
        // produces unanimous agreement on a valid cert.
        struct MirrorKernel;
        impl KernelChecker for MirrorKernel {
            fn name(&self) -> &'static str {
                "mirror_synthetic"
            }
            fn description(&self) -> &'static str {
                "synthetic — mirrors trusted-base"
            }
            fn verify(&self, cert: &Certificate) -> Result<(), CheckError> {
                cert.verify()
            }
        }
        let mut r = KernelRegistry::default();
        r.register(MirrorKernel);
        assert_eq!(r.len(), 3);
        let v = r.verify_all(&polymorphic_identity());
        assert!(matches!(v.agreement, AgreementVerdict::Unanimous));
        assert_eq!(v.outcomes.len(), 3);
    }
}
