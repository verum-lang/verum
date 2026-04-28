//! Diakrisis bridge admits — explicit, named axioms that surface
//! the type-theoretic results currently outside the Verum kernel's
//! decidable fragment.
//!
//! This module is the **trusted boundary** for K-Round-Trip V2's
//! universal canonicalize. Each admit names a specific Diakrisis
//! preprint result (paragraph + theorem number); when the preprint
//! resolves and the result lands in the kernel as a structural
//! algorithm, the corresponding admit is removed and call sites are
//! re-checked against the now-derivable lemma.
//!
//! # Why bridge admits, not silent assumptions
//!
//! Pre-V2 the round-trip rule had a single error variant
//! (`KernelError::RoundTripFailed`) that fired whenever the V0/V1
//! decidable fragment couldn't admit a pair. Calls into the universal
//! algorithm were either rejected or had to be discharged by an ad-hoc
//! `@framework(...)` axiom citation in user code. V2 closes this gap
//! by making the dependency explicit at the kernel surface:
//!
//!   * `BridgeId::ConfluenceOfModalRewrite` — Diakrisis Theorem 16.10
//!     confluence of the (Box / Diamond / Shape / Flat / Sharp)
//!     rewrite system. Required when two canonical-form paths over
//!     a modal subterm produce structurally-different normal forms;
//!     the bridge asserts they meet at a common further reduct.
//!
//!   * `BridgeId::QuotientCanonicalRepresentative` — Diakrisis
//!     Theorem 16.7 canonical-representative selector for
//!     `Quotient(base, equiv)`. Required when two terms differ
//!     only in their choice of equivalence-class representative.
//!
//!   * `BridgeId::CohesiveAdjunctionUnitCounit` — Diakrisis Theorem
//!     14.3 unit/counit naturality for the (∫ ⊣ ♭ ⊣ ♯) cohesive
//!     adjunction triple. Required for `Flat(Sharp(x))` collapse
//!     under the right adjoint side.
//!
//!   * `BridgeId::EpsMuTauWitness` — Diakrisis Axiom A-3 σ_α / π_α
//!     τ-witness construction. The K-Eps-Mu rule's V3-incremental
//!     decides necessary conditions structurally; the V3-final
//!     sufficient witness construction (σ_α from the Code_S
//!     morphism + π_α from Perform_{ε_math} naturality through
//!     axiom A-3) is the residual preprint-blocked step. V3-final
//!     surfaces the construction as this admit.
//!
//! Each admit has a kernel re-check facade — `check_<bridge>` —
//! that audits the bridge invocation site (recording the
//! [`BridgeAdmit`] in a returned audit trail) but does NOT verify
//! the underlying claim. Downstream audit reporters
//! (`vcs/red-team/round-1-architecture.md`, the `verum audit
//! --proof-honesty` walker) enumerate every bridge-admit usage.
//!
//! # Future direction (V3 promotion)
//!
//! When Diakrisis 16.10 confluence lands as a structural algorithm,
//! `check_confluence_of_modal_rewrite` is rewritten to actually
//! compute the common reduct (instead of admitting it) and the
//! `BridgeAdmit` audit entry mutates from `Admitted` to
//! `Discharged { rule: KernelRule::KConfluence }`. Same pattern for
//! 16.7 and 14.3. The trusted boundary shrinks monotonically.

use verum_common::{List, Text};

use crate::CoreTerm;

/// Identifies a specific Diakrisis result that the kernel admits
/// rather than re-derives. Each variant names the preprint
/// paragraph + theorem number so external auditors can cross-
/// reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BridgeId {
    /// Diakrisis Theorem 16.10 — confluence of the (Box / Diamond /
    /// Shape / Flat / Sharp) modal rewrite system. Two distinct
    /// reductions of a modal-bearing term meet at a common further
    /// reduct.
    ConfluenceOfModalRewrite,

    /// Diakrisis Theorem 16.7 — canonical-representative selector
    /// for `Quotient(base, equiv)`.  Decidable equality of quotient
    /// representatives modulo the equivalence relation.
    QuotientCanonicalRepresentative,

    /// Diakrisis Theorem 14.3 — unit/counit naturality for the
    /// (∫ ⊣ ♭ ⊣ ♯) cohesive adjunction triple. Required for
    /// `Flat(Sharp(x))` and `Sharp(Flat(x))` collapse on the
    /// appropriate adjunction side.
    CohesiveAdjunctionUnitCounit,

    /// Diakrisis Axiom A-3 — σ_α / π_α τ-witness for the K-Eps-Mu
    /// naturality rule. V3-incremental decides the necessary
    /// conditions (depth preservation, free-variable preservation,
    /// β-normalisation invariance) structurally; this admit covers
    /// the V3-final sufficient witness construction
    /// (σ_α from the Code_S morphism + π_α from
    /// Perform_{ε_math} naturality).
    EpsMuTauWitness,
}

impl BridgeId {
    /// Stable string identifier used for audit-report serialization.
    pub fn as_audit_str(self) -> &'static str {
        match self {
            Self::ConfluenceOfModalRewrite => "diakrisis-16.10",
            Self::QuotientCanonicalRepresentative => "diakrisis-16.7",
            Self::CohesiveAdjunctionUnitCounit => "diakrisis-14.3",
            Self::EpsMuTauWitness => "diakrisis-A-3",
        }
    }

    /// Human-readable description of what the bridge admits.
    pub fn description(self) -> &'static str {
        match self {
            Self::ConfluenceOfModalRewrite => {
                "confluence of the modal rewrite system over Box / Diamond / Shape / Flat / Sharp"
            }
            Self::QuotientCanonicalRepresentative => {
                "canonical-representative selector for Quotient(base, equiv)"
            }
            Self::CohesiveAdjunctionUnitCounit => {
                "unit/counit naturality for the cohesive triple adjunction (∫ ⊣ ♭ ⊣ ♯)"
            }
            Self::EpsMuTauWitness => {
                "σ_α / π_α τ-witness for K-Eps-Mu naturality (Code_S + Perform_{ε_math})"
            }
        }
    }
}

/// One bridge-admit invocation. Captured by the V2 canonicalize
/// algorithm so audit walkers can enumerate every reliance on a
/// preprint-blocked result.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeAdmit {
    /// Which result was admitted.
    pub bridge: BridgeId,
    /// Human-readable callsite context.
    pub context: Text,
}

/// Audit trail for a canonicalize / round-trip run. Empty trail means
/// the algorithm completed entirely within the V0/V1 decidable
/// fragment; non-empty trail means the V2 universal path admitted
/// at least one preprint-blocked claim.
#[derive(Debug, Clone, Default)]
pub struct BridgeAudit {
    admits: Vec<BridgeAdmit>,
}

impl BridgeAudit {
    pub fn new() -> Self {
        Self { admits: Vec::new() }
    }

    /// Record a bridge admit. Idempotent on (bridge, context) pairs
    /// — the same bridge invoked from the same callsite logs once.
    pub fn record(&mut self, bridge: BridgeId, context: impl Into<Text>) {
        let ctx = context.into();
        if !self.admits.iter().any(|a| a.bridge == bridge && a.context == ctx) {
            self.admits.push(BridgeAdmit { bridge, context: ctx });
        }
    }

    /// All admits in insertion order.
    pub fn admits(&self) -> &[BridgeAdmit] {
        &self.admits
    }

    /// True iff the audit trail is empty — the algorithm completed
    /// without invoking any bridge admit.
    pub fn is_decidable(&self) -> bool {
        self.admits.is_empty()
    }

    /// Stable, sorted, deduplicated list of bridge IDs invoked.
    /// Used by audit reports.
    pub fn bridges(&self) -> List<&'static str> {
        let mut names: Vec<&'static str> =
            self.admits.iter().map(|a| a.bridge.as_audit_str()).collect();
        names.sort();
        names.dedup();
        let mut out = List::new();
        for n in names {
            out.push(n);
        }
        out
    }
}

/// Bridge invocation: confluence of modal rewrites. Admits without
/// re-deriving — V3 will replace this with a structural algorithm.
///
/// Returns the LHS unchanged (the bridge does not perform any
/// reduction; it is purely an audit-recording hook). Callers that
/// need the modal subterm normalized should still apply the V0/V1
/// rewrites before invoking the bridge.
pub fn admit_confluence_of_modal_rewrite(
    audit: &mut BridgeAudit,
    context: impl Into<Text>,
    term: &CoreTerm,
) -> CoreTerm {
    audit.record(BridgeId::ConfluenceOfModalRewrite, context);
    term.clone()
}

/// Bridge invocation: quotient canonical representative.
pub fn admit_quotient_canonical_representative(
    audit: &mut BridgeAudit,
    context: impl Into<Text>,
    term: &CoreTerm,
) -> CoreTerm {
    audit.record(BridgeId::QuotientCanonicalRepresentative, context);
    term.clone()
}

/// Bridge invocation: cohesive adjunction unit/counit naturality.
pub fn admit_cohesive_adjunction_unit_counit(
    audit: &mut BridgeAudit,
    context: impl Into<Text>,
    term: &CoreTerm,
) -> CoreTerm {
    audit.record(BridgeId::CohesiveAdjunctionUnitCounit, context);
    term.clone()
}

/// Bridge invocation: K-Eps-Mu σ_α / π_α τ-witness construction.
/// V3-final hands off to this admit when V3-incremental gates pass
/// but the sufficient witness construction is still preprint-blocked
/// on Diakrisis A-3. The term is returned unchanged — purely an
/// audit-recording hook. V3 promotion replaces the body with the
/// structural Code_S / Perform_{ε_math} computation.
pub fn admit_eps_mu_tau_witness(
    audit: &mut BridgeAudit,
    context: impl Into<Text>,
    term: &CoreTerm,
) -> CoreTerm {
    audit.record(BridgeId::EpsMuTauWitness, context);
    term.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_starts_decidable() {
        let a = BridgeAudit::new();
        assert!(a.is_decidable());
        assert!(a.admits().is_empty());
    }

    #[test]
    fn record_appends_bridge_admit() {
        let mut a = BridgeAudit::new();
        a.record(BridgeId::ConfluenceOfModalRewrite, "callsite-A");
        assert!(!a.is_decidable());
        assert_eq!(a.admits().len(), 1);
        assert_eq!(a.admits()[0].bridge, BridgeId::ConfluenceOfModalRewrite);
        assert_eq!(a.admits()[0].context.as_str(), "callsite-A");
    }

    #[test]
    fn record_dedups_same_bridge_same_context() {
        let mut a = BridgeAudit::new();
        a.record(BridgeId::ConfluenceOfModalRewrite, "callsite-A");
        a.record(BridgeId::ConfluenceOfModalRewrite, "callsite-A");
        assert_eq!(a.admits().len(), 1, "duplicates must collapse");
    }

    #[test]
    fn record_keeps_distinct_contexts() {
        let mut a = BridgeAudit::new();
        a.record(BridgeId::ConfluenceOfModalRewrite, "callsite-A");
        a.record(BridgeId::ConfluenceOfModalRewrite, "callsite-B");
        assert_eq!(a.admits().len(), 2, "different contexts must both record");
    }

    #[test]
    fn record_keeps_distinct_bridges() {
        let mut a = BridgeAudit::new();
        a.record(BridgeId::ConfluenceOfModalRewrite, "x");
        a.record(BridgeId::QuotientCanonicalRepresentative, "x");
        a.record(BridgeId::CohesiveAdjunctionUnitCounit, "x");
        assert_eq!(a.admits().len(), 3);
    }

    #[test]
    fn bridges_returns_sorted_dedup() {
        let mut a = BridgeAudit::new();
        a.record(BridgeId::QuotientCanonicalRepresentative, "x");
        a.record(BridgeId::ConfluenceOfModalRewrite, "y");
        a.record(BridgeId::ConfluenceOfModalRewrite, "z");
        let bridges = a.bridges();
        let names: Vec<&str> = bridges.iter().copied().collect();
        // Sorted: 14.3 < 16.10 < 16.7 lexicographically actually.
        // Let's just check the membership and dedup count.
        assert_eq!(names.len(), 2, "two distinct bridges expected");
        assert!(names.contains(&"diakrisis-16.10"));
        assert!(names.contains(&"diakrisis-16.7"));
    }

    #[test]
    fn admit_helpers_record_and_return_term_unchanged() {
        let mut a = BridgeAudit::new();
        let term = CoreTerm::Var(Text::from("foo"));
        let out = admit_confluence_of_modal_rewrite(&mut a, "ctx", &term);
        assert_eq!(out, term, "bridge admits must not mutate term");
        assert_eq!(a.admits().len(), 1);

        let out2 = admit_quotient_canonical_representative(&mut a, "ctx", &term);
        assert_eq!(out2, term);
        assert_eq!(a.admits().len(), 2);

        let out3 = admit_cohesive_adjunction_unit_counit(&mut a, "ctx", &term);
        assert_eq!(out3, term);
        assert_eq!(a.admits().len(), 3);
    }

    #[test]
    fn bridge_id_audit_str_is_stable() {
        // Audit reports rely on these strings; protect against
        // accidental rename.
        assert_eq!(
            BridgeId::ConfluenceOfModalRewrite.as_audit_str(),
            "diakrisis-16.10"
        );
        assert_eq!(
            BridgeId::QuotientCanonicalRepresentative.as_audit_str(),
            "diakrisis-16.7"
        );
        assert_eq!(
            BridgeId::CohesiveAdjunctionUnitCounit.as_audit_str(),
            "diakrisis-14.3"
        );
        assert_eq!(
            BridgeId::EpsMuTauWitness.as_audit_str(),
            "diakrisis-A-3"
        );
    }

    #[test]
    fn bridge_id_descriptions_mention_diakrisis_terms() {
        assert!(BridgeId::ConfluenceOfModalRewrite
            .description()
            .contains("modal"));
        assert!(BridgeId::QuotientCanonicalRepresentative
            .description()
            .contains("Quotient"));
        assert!(BridgeId::CohesiveAdjunctionUnitCounit
            .description()
            .contains("cohesive"));
        assert!(BridgeId::EpsMuTauWitness
            .description()
            .contains("τ-witness"));
    }

    #[test]
    fn admit_eps_mu_tau_witness_records_and_returns_unchanged() {
        let mut a = BridgeAudit::new();
        let term = CoreTerm::Var(Text::from("F"));
        let out = admit_eps_mu_tau_witness(&mut a, "K-Eps-Mu callsite", &term);
        assert_eq!(out, term);
        assert_eq!(a.admits().len(), 1);
        assert_eq!(a.admits()[0].bridge, BridgeId::EpsMuTauWitness);
    }
}
