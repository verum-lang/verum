//! Framework-compatibility matrix — V0.
//!
//! metatheory table delegates "axiom bundle's mutual
//! consistency" to "the bundle's citation" — an honest soundness
//! escape hatch. A user who imports two contradictory framework
//! packages (e.g., `core.math.frameworks.classical_lem` +
//! `core.math.frameworks.univalence` with the wrong cube
//! interpretation) gets `False` in their kernel without warning.
//!
//! This module ports the well-known compatibility table from
//! corpus folklore (Coquand 1990 paradox; HoTT Book chapter 4;
//! Awodey-Bauer-Hofmann 2018) into a Rust data structure that the
//! verification pipeline can query.
//!
//! # V0 surface
//!
//!   * [`IncompatiblePair`] — one (framework_a, framework_b)
//!     conflict, with reason + literature citation.
//!   * [`KNOWN_INCOMPATIBLE_PAIRS`] — the static table. V0 ships
//!     four documented incompatibilities.
//!   * [`audit_framework_set`] — given a list of corpus identifiers
//!     used in a module, return a diagnostic for every incompatible
//!     pair found.
//!
//! V1 will:
//!   * Wire `audit_framework_set` into `HygieneRecheckPass` so the
//!     check fires on every `verum verify`.
//!   * Provide a CLI surface `verum audit --framework-conflicts`.
//!   * Accept stdlib-side declarative additions (each framework
//!     package can declare its own conflicts in
//!     `core.math.frameworks.<name>::CONFLICTS_WITH`).

use verum_common::Text;

use crate::framework_hygiene::{HygieneDiagnostic, HygieneSeverity};

/// One framework-compatibility violation. Symmetric in `framework_a`
/// and `framework_b` — the audit treats `(A, B)` and `(B, A)` as the
/// same conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncompatiblePair {
    /// First framework corpus identifier.
    pub framework_a: &'static str,
    /// Second framework corpus identifier.
    pub framework_b: &'static str,
    /// Short human-readable reason ("Coquand's paradox", "UIP
    /// contradicts univalence", etc.).
    pub reason: &'static str,
    /// Literature citation locating the formal proof of the
    /// inconsistency.
    pub citation: &'static str,
}

/// V0 catalogue of well-known incompatibilities. Conservative
/// non-controversial entries only — entries that have a clear
/// literature citation.
///
/// Adding to this table is a *trust-boundary action*: every entry
/// should reference a peer-reviewed result (either directly or via
/// a textbook reproduction). Stdlib authors can extend this set
/// via the V1 declarative-conflicts surface (deferred).
pub const KNOWN_INCOMPATIBLE_PAIRS: &[IncompatiblePair] = &[
    // UIP ⊥ Univalence: the canonical clash. UIP says any two
    // proofs of the same equality are equal; univalence says paths
    // in `Type` correspond to equivalences (which can be non-trivial,
    // e.g. swap : Bool ≃ Bool). The kernel already rejects
    // UIP-shape axioms via KernelError::UipForbidden — this entry
    // is for completeness on the symmetric dual (importing both
    // packages is a no-op since UIP would be rejected, but the
    // diagnostic helps the user understand why).
    IncompatiblePair {
        framework_a: "uip",
        framework_b: "univalence",
        reason: "UIP contradicts univalence — paths in Type are equivalences",
        citation: "HoTT Book 2013, Theorem 7.2.1 + Awodey-Bauer 2004",
    },

    // Impredicative Prop + classical LEM + univalence ⊥
    // (Coquand 1990 paradox). The combination of any two is fine;
    // all three together derive `False` via Coquand's classical
    // impredicative inhabitation argument.
    IncompatiblePair {
        framework_a: "impredicative_prop",
        framework_b: "classical_lem_with_univalence",
        reason: "Coquand 1990 paradox — impredicative Prop + classical LEM + univalence is inconsistent",
        citation: "Coquand T. 1990. The paradox of trees in Type Theory; HoTT Book 2013, §3.2",
    },

    // Anti-classical + classical LEM. Anti-classical packages
    // postulate `¬LEM` (e.g. for constructive analysis foundations);
    // importing alongside classical_lem yields `False`.
    IncompatiblePair {
        framework_a: "anti_classical",
        framework_b: "classical_lem",
        reason: "anti-classical postulates ¬LEM; classical_lem postulates LEM",
        citation: "Bishop E. 1967. Foundations of Constructive Analysis; ¬LEM ∧ LEM ⊢ ⊥",
    },

    // K-axiom (intensional MLTT) + univalence. K is a generalisation
    // of UIP for inductive types; the same incompatibility applies.
    IncompatiblePair {
        framework_a: "k_axiom",
        framework_b: "univalence",
        reason: "K-axiom (UIP for inductives) contradicts univalence",
        citation: "Hofmann M. 1995. Extensional Constructs in Intensional Type Theory, §5.5",
    },
];

/// Audit a set of framework corpus identifiers (typically the
/// distinct corpora found in a module's `@framework(...)`
/// annotations) against [`KNOWN_INCOMPATIBLE_PAIRS`]. Returns one
/// `HygieneDiagnostic` per incompatible pair found.
///
/// Severity is always `Error` — a pair listed in the matrix has a
/// formal proof of inconsistency; admitting both packages derives
/// `False` and breaks every theorem in the module.
///
/// O(n × m) where n = corpus count and m = matrix size. Matrix is
/// O(10) entries today, so the audit is effectively O(n).
pub fn audit_framework_set(corpora: &[Text]) -> Vec<HygieneDiagnostic> {
    let mut out = Vec::new();
    let names: Vec<&str> = corpora.iter().map(|t| t.as_str()).collect();
    for pair in KNOWN_INCOMPATIBLE_PAIRS {
        let has_a = names.iter().any(|n| *n == pair.framework_a);
        let has_b = names.iter().any(|n| *n == pair.framework_b);
        if has_a && has_b {
            out.push(HygieneDiagnostic {
                rule: "R4",
                severity: HygieneSeverity::Error,
                message: Text::from(format!(
                    "framework conflict: {} ⊥ {} — {} ({})",
                    pair.framework_a,
                    pair.framework_b,
                    pair.reason,
                    pair.citation,
                )),
            });
        }
    }
    out
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_set_audits_clean() {
        assert!(audit_framework_set(&[]).is_empty());
    }

    #[test]
    fn single_framework_audits_clean() {
        assert!(audit_framework_set(&[Text::from("lurie_htt")]).is_empty());
    }

    #[test]
    fn compatible_pair_audits_clean() {
        let corpora = vec![Text::from("lurie_htt"), Text::from("schreiber_dcct")];
        assert!(audit_framework_set(&corpora).is_empty());
    }

    #[test]
    fn uip_plus_univalence_rejected() {
        let corpora = vec![Text::from("uip"), Text::from("univalence")];
        let diagnostics = audit_framework_set(&corpora);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, HygieneSeverity::Error);
        assert_eq!(diagnostics[0].rule, "R4");
        assert!(diagnostics[0].message.as_str().contains("uip"));
        assert!(diagnostics[0].message.as_str().contains("univalence"));
        assert!(diagnostics[0].message.as_str().contains("HoTT Book"));
    }

    #[test]
    fn order_does_not_matter() {
        let cs1 = vec![Text::from("uip"), Text::from("univalence")];
        let cs2 = vec![Text::from("univalence"), Text::from("uip")];
        assert_eq!(
            audit_framework_set(&cs1).len(),
            audit_framework_set(&cs2).len()
        );
    }

    #[test]
    fn anti_classical_plus_classical_lem_rejected() {
        let corpora = vec![
            Text::from("anti_classical"),
            Text::from("classical_lem"),
        ];
        let diagnostics = audit_framework_set(&corpora);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, HygieneSeverity::Error);
        assert!(diagnostics[0].message.as_str().contains("Bishop"));
    }

    #[test]
    fn k_axiom_plus_univalence_rejected() {
        let corpora = vec![Text::from("k_axiom"), Text::from("univalence")];
        let diagnostics = audit_framework_set(&corpora);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.as_str().contains("Hofmann"));
    }

    #[test]
    fn coquand_triple_conflict() {
        // V0 records `impredicative_prop ⊥ classical_lem_with_univalence`
        // as a fused pair (the third dependency lives in the
        // `classical_lem_with_univalence` package name itself).
        let corpora = vec![
            Text::from("impredicative_prop"),
            Text::from("classical_lem_with_univalence"),
        ];
        let diagnostics = audit_framework_set(&corpora);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.as_str().contains("Coquand 1990"));
    }

    #[test]
    fn multiple_conflicts_each_reported_separately() {
        // A module that imports a quartet of conflicting packages
        // should get one diagnostic per conflict pair, not just one
        // aggregate.
        let corpora = vec![
            Text::from("uip"),
            Text::from("univalence"),
            Text::from("anti_classical"),
            Text::from("classical_lem"),
        ];
        let diagnostics = audit_framework_set(&corpora);
        // uip⊥univalence + anti_classical⊥classical_lem = 2
        assert_eq!(diagnostics.len(), 2);
    }

    #[test]
    fn known_incompatible_pairs_table_well_formed() {
        // No self-conflicts.
        for pair in KNOWN_INCOMPATIBLE_PAIRS {
            assert_ne!(pair.framework_a, pair.framework_b);
            assert!(!pair.framework_a.is_empty());
            assert!(!pair.framework_b.is_empty());
            assert!(!pair.reason.is_empty());
            assert!(!pair.citation.is_empty());
        }
        // No duplicates (treating (a,b) == (b,a)).
        for (i, p1) in KNOWN_INCOMPATIBLE_PAIRS.iter().enumerate() {
            for p2 in &KNOWN_INCOMPATIBLE_PAIRS[i + 1..] {
                let same = (p1.framework_a == p2.framework_a
                    && p1.framework_b == p2.framework_b)
                    || (p1.framework_a == p2.framework_b
                        && p1.framework_b == p2.framework_a);
                assert!(
                    !same,
                    "duplicate conflict entry: ({}, {})",
                    p1.framework_a, p1.framework_b
                );
            }
        }
    }
}
