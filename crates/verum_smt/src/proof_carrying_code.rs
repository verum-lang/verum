//! Proof-Carrying Code (PCC) — formal proofs as bytecode metadata.
//!
//! Necula's proof-carrying code (1997) attaches a *machine-checkable
//! proof* of a safety property to compiled bytecode. The recipient
//! verifies the proof rather than re-running the analysis,
//! enabling third-party safety audits without trusting the
//! compiler that produced the binary.
//!
//! This module provides the **bundle layer**: serializable proof
//! certificates keyed by a content-addressed hash, attached to a
//! VBC artifact, with audit-trail metadata (who proved it, when,
//! against which goal, with which solver). The actual SMT proof
//! objects live in `crate::certificates`; this module is concerned
//! with bundling, lookup, and integrity.
//!
//! ## Bundle structure
//!
//! ```text
//!     ProofBundle {
//!         certificates: Map<GoalHash, ProofCertificate>,
//!         metadata:     BundleMetadata,
//!     }
//! ```
//!
//! Each certificate is keyed by a stable hash of its goal so that
//! the verifier can quickly check "do we have a proof of *this*
//! goal?" without re-rendering the whole proof.
//!
//! ## Trust model
//!
//! A bundle is **only** as trustworthy as the proof certificates
//! it contains. PCC does not avoid trust — it relocates trust from
//! the compiler-pipeline to the proof-checker, which is small and
//! auditable.
//!
//! ## Status
//!
//! Standalone bundle/serialisation core. Wiring `@verify(formal)`
//! to populate the bundle during the verification phase, and
//! adding a `verum verify-bundle` CLI command, are future
//! integration steps.

use std::collections::HashMap;

use verum_common::Text;

/// A goal hash — content-addressed identifier of a verification
/// obligation. Stable across runs that produce the same goal.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GoalHash {
    pub digest: Text,
}

impl GoalHash {
    pub fn new(digest: impl Into<Text>) -> Self {
        Self {
            digest: digest.into(),
        }
    }

    /// Compute a deterministic hash from a goal's textual
    /// representation. Uses a simple FNV-1a 64-bit digest;
    /// production deployments should swap this for SHA-256.
    pub fn from_goal(goal_text: &str) -> Self {
        let mut hash: u64 = 0xcbf29ce484222325;
        for byte in goal_text.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        Self {
            digest: Text::from(format!("{:016x}", hash)),
        }
    }
}

impl std::fmt::Display for GoalHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.digest.as_str())
    }
}

/// A single proof certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofCertificate {
    /// The goal that was proved (textual, for human audit).
    pub goal_text: Text,
    /// The solver that produced the proof.
    pub solver: Text,
    /// Solver-specific proof object (e.g., Z3 proof tree as text).
    pub proof_object: Text,
    /// Wall-clock duration of the proof in milliseconds.
    pub duration_ms: u64,
    /// ISO-8601 timestamp string, opaque to this module.
    pub timestamp: Text,
}

impl ProofCertificate {
    pub fn new(
        goal_text: impl Into<Text>,
        solver: impl Into<Text>,
        proof_object: impl Into<Text>,
        duration_ms: u64,
        timestamp: impl Into<Text>,
    ) -> Self {
        Self {
            goal_text: goal_text.into(),
            solver: solver.into(),
            proof_object: proof_object.into(),
            duration_ms,
            timestamp: timestamp.into(),
        }
    }

    /// Compute the goal hash for this certificate.
    pub fn goal_hash(&self) -> GoalHash {
        GoalHash::from_goal(self.goal_text.as_str())
    }
}

/// Audit-trail metadata for the whole bundle.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BundleMetadata {
    /// Compiler version that produced this bundle.
    pub compiler_version: Text,
    /// Source file or module path the bundle pertains to.
    pub source_path: Text,
    /// Total number of certificates contained.
    pub certificate_count: usize,
    /// Sum of all certificate durations (ms).
    pub total_duration_ms: u64,
}

/// A complete proof-carrying bundle.
#[derive(Debug, Clone, Default)]
pub struct ProofBundle {
    certificates: HashMap<GoalHash, ProofCertificate>,
    metadata: BundleMetadata,
}

impl ProofBundle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a certificate. If a certificate already exists for this
    /// goal hash with a *different* proof, returns
    /// `Err(BundleError::Conflict)` — same goal can't have two
    /// distinct proofs in the same bundle.
    pub fn add(
        &mut self,
        cert: ProofCertificate,
    ) -> Result<(), BundleError> {
        let h = cert.goal_hash();
        if let Some(existing) = self.certificates.get(&h) {
            if existing != &cert {
                return Err(BundleError::Conflict {
                    goal_hash: h,
                });
            }
            return Ok(());
        }
        self.metadata.certificate_count += 1;
        self.metadata.total_duration_ms = self
            .metadata
            .total_duration_ms
            .saturating_add(cert.duration_ms);
        self.certificates.insert(h, cert);
        Ok(())
    }

    /// Look up a certificate by goal hash.
    pub fn lookup(&self, hash: &GoalHash) -> Option<&ProofCertificate> {
        self.certificates.get(hash)
    }

    /// Look up a certificate by goal text (computes hash internally).
    pub fn lookup_by_goal(&self, goal_text: &str) -> Option<&ProofCertificate> {
        self.lookup(&GoalHash::from_goal(goal_text))
    }

    /// Number of certificates in the bundle.
    pub fn len(&self) -> usize {
        self.certificates.len()
    }

    pub fn is_empty(&self) -> bool {
        self.certificates.is_empty()
    }

    pub fn metadata(&self) -> &BundleMetadata {
        &self.metadata
    }

    pub fn set_metadata_compiler(&mut self, version: impl Into<Text>) {
        self.metadata.compiler_version = version.into();
    }

    pub fn set_metadata_source(&mut self, path: impl Into<Text>) {
        self.metadata.source_path = path.into();
    }

    /// Iterate over certificates in deterministic alphabetical
    /// order by goal-hash digest, suitable for proof_stability.
    pub fn iter_sorted(&self) -> impl Iterator<Item = (&GoalHash, &ProofCertificate)> {
        let mut hashes: Vec<&GoalHash> = self.certificates.keys().collect();
        hashes.sort_by(|a, b| a.digest.as_str().cmp(b.digest.as_str()));
        hashes.into_iter().map(move |h| (h, &self.certificates[h]))
    }

    /// Verify integrity: every stored certificate's goal_text
    /// must hash to its key. Returns the first mismatch found, or
    /// `Ok(())` if every entry is consistent.
    pub fn check_integrity(&self) -> Result<(), BundleError> {
        for (key, cert) in &self.certificates {
            let recomputed = cert.goal_hash();
            if &recomputed != key {
                return Err(BundleError::IntegrityFailure {
                    expected: key.clone(),
                    recomputed,
                });
            }
        }
        Ok(())
    }
}

/// Errors raised when manipulating a bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleError {
    /// Two distinct certificates were registered under the same
    /// goal hash — indicates a buggy compiler producing two
    /// different proofs of the same goal.
    Conflict { goal_hash: GoalHash },
    /// A certificate's stored goal text no longer hashes to its
    /// key — indicates the bundle has been tampered with.
    IntegrityFailure {
        expected: GoalHash,
        recomputed: GoalHash,
    },
}

impl std::fmt::Display for BundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict { goal_hash } => write!(
                f,
                "two distinct proofs registered for goal hash `{}`",
                goal_hash
            ),
            Self::IntegrityFailure {
                expected,
                recomputed,
            } => write!(
                f,
                "PCC integrity failure: stored hash `{}` but goal hashes to `{}`",
                expected, recomputed
            ),
        }
    }
}

impl std::error::Error for BundleError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn cert(goal: &str, solver: &str, proof: &str) -> ProofCertificate {
        ProofCertificate::new(goal, solver, proof, 100, "2026-04-13T00:00:00Z")
    }

    #[test]
    fn goal_hash_is_deterministic() {
        let h1 = GoalHash::from_goal("forall x. P(x)");
        let h2 = GoalHash::from_goal("forall x. P(x)");
        assert_eq!(h1, h2);
    }

    #[test]
    fn distinct_goals_hash_differently() {
        let h1 = GoalHash::from_goal("forall x. P(x)");
        let h2 = GoalHash::from_goal("forall x. Q(x)");
        assert_ne!(h1, h2);
    }

    #[test]
    fn empty_bundle_has_no_certs() {
        let b = ProofBundle::new();
        assert!(b.is_empty());
        assert_eq!(b.len(), 0);
        assert_eq!(b.metadata().certificate_count, 0);
    }

    #[test]
    fn add_cert_increments_count_and_duration() {
        let mut b = ProofBundle::new();
        let c = cert("x = x", "z3", "refl");
        b.add(c).unwrap();
        assert_eq!(b.len(), 1);
        assert_eq!(b.metadata().certificate_count, 1);
        assert_eq!(b.metadata().total_duration_ms, 100);
    }

    #[test]
    fn lookup_returns_added_cert() {
        let mut b = ProofBundle::new();
        let c = cert("x = x", "z3", "refl");
        b.add(c.clone()).unwrap();
        let found = b.lookup_by_goal("x = x");
        assert!(found.is_some());
        assert_eq!(found.unwrap().solver.as_str(), "z3");
    }

    #[test]
    fn lookup_missing_goal_returns_none() {
        let b = ProofBundle::new();
        assert!(b.lookup_by_goal("never_added").is_none());
    }

    #[test]
    fn re_adding_same_cert_is_idempotent() {
        let mut b = ProofBundle::new();
        let c = cert("x = x", "z3", "refl");
        b.add(c.clone()).unwrap();
        b.add(c).unwrap();
        assert_eq!(b.len(), 1);
        assert_eq!(b.metadata().certificate_count, 1);
    }

    #[test]
    fn conflicting_cert_for_same_goal_rejected() {
        let mut b = ProofBundle::new();
        b.add(cert("x = x", "z3", "refl_a")).unwrap();
        let err = b.add(cert("x = x", "z3", "refl_b_DIFFERENT"));
        assert!(matches!(err, Err(BundleError::Conflict { .. })));
    }

    #[test]
    fn metadata_setters_update_in_place() {
        let mut b = ProofBundle::new();
        b.set_metadata_compiler("verum-0.4.0");
        b.set_metadata_source("src/main.vr");
        assert_eq!(b.metadata().compiler_version.as_str(), "verum-0.4.0");
        assert_eq!(b.metadata().source_path.as_str(), "src/main.vr");
    }

    #[test]
    fn iter_sorted_yields_alphabetical_order() {
        let mut b = ProofBundle::new();
        b.add(cert("zebra_goal", "z3", "p1")).unwrap();
        b.add(cert("alpha_goal", "z3", "p2")).unwrap();
        b.add(cert("mu_goal", "z3", "p3")).unwrap();

        let order: Vec<String> = b
            .iter_sorted()
            .map(|(h, _)| h.digest.as_str().to_string())
            .collect();
        let mut sorted_check = order.clone();
        sorted_check.sort();
        assert_eq!(order, sorted_check);
    }

    #[test]
    fn integrity_check_passes_on_clean_bundle() {
        let mut b = ProofBundle::new();
        b.add(cert("x = x", "z3", "refl")).unwrap();
        b.add(cert("y > 0", "z3", "lin_arith")).unwrap();
        assert!(b.check_integrity().is_ok());
    }

    #[test]
    fn total_duration_sums_cert_durations() {
        let mut b = ProofBundle::new();
        b.add(ProofCertificate::new(
            "g1", "z3", "p1", 50, "t",
        ))
        .unwrap();
        b.add(ProofCertificate::new(
            "g2", "z3", "p2", 250, "t",
        ))
        .unwrap();
        b.add(ProofCertificate::new(
            "g3", "z3", "p3", 700, "t",
        ))
        .unwrap();
        assert_eq!(b.metadata().total_duration_ms, 1000);
    }

    #[test]
    fn duration_saturates_on_overflow() {
        let mut b = ProofBundle::new();
        b.add(ProofCertificate::new(
            "g1", "z3", "p1", u64::MAX, "t",
        ))
        .unwrap();
        b.add(ProofCertificate::new(
            "g2", "z3", "p2", 100, "t",
        ))
        .unwrap();
        assert_eq!(b.metadata().total_duration_ms, u64::MAX);
    }

    #[test]
    fn distinct_goals_can_coexist() {
        let mut b = ProofBundle::new();
        b.add(cert("g1", "z3", "p1")).unwrap();
        b.add(cert("g2", "z3", "p2")).unwrap();
        b.add(cert("g3", "z3", "p3")).unwrap();
        assert_eq!(b.len(), 3);
    }

    #[test]
    fn goal_hash_display_is_hex_digest() {
        let h = GoalHash::from_goal("test");
        let s = format!("{}", h);
        assert_eq!(s.len(), 16); // 64-bit hex
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
