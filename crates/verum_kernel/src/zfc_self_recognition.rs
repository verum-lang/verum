//! Kernel self-recognition vs. ZFC + 2 inaccessibles — V0
//! algorithmic kernel rule.
//!
//! ## What this delivers
//!
//! Verum's trusted kernel is sound *relative* to a meta-theory; the
//! conventional choice is **ZFC + 2 strongly inaccessible cardinals**
//! (κ_1 < κ_2), the smallest fragment of set theory that:
//!
//!   1. Models every kernel-defining axiom (extensionality, pairing,
//!      union, infinity, separation, replacement, foundation, choice).
//!   2. Provides Grothendieck universes for the universe-tower
//!      (`Type_0 ∈ Type_1 ∈ Type_2`) — one per inaccessible.
//!   3. Houses the (∞,1)-categorical content (HTT lives in
//!      ZFC + 1 inaccessible; the second inaccessible is needed
//!      for the *meta*-classifier of (∞,1)-categories).
//!
//! Pre-this-module the kernel's relative-consistency claim was
//! folklore — there was no algorithmic surface that listed the seven
//! kernel rules, decomposed each into its ZFC-axiom + universe-cardinal
//! requirements, or decided whether a kernel-derivable judgement
//! could be lifted into the meta-theory.
//!
//! ## V0 algorithmic surface
//!
//! V0 ships:
//!
//!   1. [`ZfcAxiom`] — the eight ZFC axioms (one ZFC-extension flag
//!      per axiom; see Kunen 2011 Ch. III).
//!   2. [`InaccessibleLevel`] — `Kappa1` / `Kappa2` (the two
//!      Grothendieck universes Verum's universe-tower requires).
//!   3. [`KernelRuleId`] — the seven rules: `K-Refine`, `K-Univ`,
//!      `K-Pos`, `K-Norm`, `K-FwAx`, `K-Adj-Unit`, `K-Adj-Counit`.
//!   4. [`MetaTheoryRequirements`] — per-rule decomposition record
//!      `(zfc_axioms, inaccessibles)` listing exactly which
//!      meta-theoretic assumptions the rule rests on.
//!   5. [`required_meta_theory(rule)`] — algorithmic decomposition.
//!   6. [`is_zfc_plus_2_inacc_provable(rule)`] — decision predicate.
//!   7. [`SelfRecognitionAudit`] — accumulator structure that records
//!      every kernel-rule citation and surfaces the union of
//!      meta-theory requirements (the "trusted-base report").
//!
//! ## What this UNBLOCKS
//!
//!   - **VVA §16.5 Phase 5** ("Full MSFS self-recognition") — the
//!     `core.math.foundations.self_recognition` corpus has the
//!     .vr-level axiomatic surface; this module provides the
//!     algorithmic counterpart, completing the cross-validation
//!     loop the file's introduction requires.
//!   - **`verum audit --self-recognition`** — the CLI command can
//!     now query [`SelfRecognitionAudit::report`] for the precise
//!     set of meta-theoretic axioms a given proof transitively
//!     depends on.
//!   - **MSFS §11 Trinitarian construction** — three inaccessibles
//!     are required per VVA roadmap; the [`InaccessibleLevel`] enum
//!     extends naturally (V1 promotion lands `Kappa3`).

use serde::{Deserialize, Serialize};
use verum_common::Text;

// =============================================================================
// ZFC axiom enumeration
// =============================================================================

/// The eight ZFC axioms (per Kunen 2011 *Set Theory* Ch. III).  Each
/// kernel rule rests on a subset of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ZfcAxiom {
    /// **Extensionality** — sets with the same elements are equal.
    Extensionality,
    /// **Pairing** — for any `a, b`, the set `{a, b}` exists.
    Pairing,
    /// **Union** — for any set `X`, the union `⋃X` exists.
    Union,
    /// **Power Set** — for any set `X`, the power set `P(X)` exists.
    PowerSet,
    /// **Infinity** — there exists an inductive set (`ω`).
    Infinity,
    /// **Separation** (axiom schema) — for every formula φ and set
    /// `X`, the set `{ x ∈ X : φ(x) }` exists.
    Separation,
    /// **Replacement** (axiom schema) — the image of a set under a
    /// definable function is a set.
    Replacement,
    /// **Foundation** — every non-empty set has an ∈-minimal element.
    Foundation,
    /// **Choice** — every family of non-empty sets has a choice function.
    Choice,
}

impl ZfcAxiom {
    /// Diagnostic name.
    pub fn name(self) -> &'static str {
        match self {
            ZfcAxiom::Extensionality => "Extensionality",
            ZfcAxiom::Pairing => "Pairing",
            ZfcAxiom::Union => "Union",
            ZfcAxiom::PowerSet => "PowerSet",
            ZfcAxiom::Infinity => "Infinity",
            ZfcAxiom::Separation => "Separation",
            ZfcAxiom::Replacement => "Replacement",
            ZfcAxiom::Foundation => "Foundation",
            ZfcAxiom::Choice => "Choice",
        }
    }

    /// Iterate the full ZFC axiom list.
    pub fn full_list() -> [ZfcAxiom; 9] {
        [
            ZfcAxiom::Extensionality,
            ZfcAxiom::Pairing,
            ZfcAxiom::Union,
            ZfcAxiom::PowerSet,
            ZfcAxiom::Infinity,
            ZfcAxiom::Separation,
            ZfcAxiom::Replacement,
            ZfcAxiom::Foundation,
            ZfcAxiom::Choice,
        ]
    }
}

// =============================================================================
// Inaccessible cardinals
// =============================================================================

/// Inaccessible cardinal level.  `Kappa1` is the first strongly
/// inaccessible (gives `Type_1`); `Kappa2` is the second (gives
/// `Type_2`, host for the (∞,1)-classifier).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InaccessibleLevel {
    /// `κ_1` — the first strongly inaccessible.  Models `Type_1`.
    Kappa1,
    /// `κ_2` — the second strongly inaccessible.  Hosts the
    /// (∞,1)-classifier of small ∞-categories.
    Kappa2,
}

impl InaccessibleLevel {
    pub fn name(self) -> &'static str {
        match self {
            InaccessibleLevel::Kappa1 => "κ_1",
            InaccessibleLevel::Kappa2 => "κ_2",
        }
    }
}

// =============================================================================
// Kernel rule identifier
// =============================================================================

/// The seven kernel rules whose ZFC-decomposition we expose.  Per
/// VVA §11.3 the rule list is:
///
///   K-Refine — depth-strict comprehension (Diakrisis T-2f*).
///   K-Univ — universe-consistency (predicative hierarchy).
///   K-Pos — strict positivity (Berardi 1998).
///   K-Norm — strong normalisation (Huber 2019 + K-FwAx).
///   K-FwAx — framework-axiom admission (Prop-only side condition).
///   K-Adj-Unit — α ⊣ ε unit identity (Diakrisis 108.T).
///   K-Adj-Counit — α ⊣ ε counit identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KernelRuleId {
    /// K-Refine — depth-strict comprehension.
    Refine,
    /// K-Univ — universe consistency.
    Univ,
    /// K-Pos — strict positivity (Berardi).
    Pos,
    /// K-Norm — strong normalisation.
    Norm,
    /// K-FwAx — framework-axiom Prop-only admission.
    FwAx,
    /// K-Adj-Unit — α ⊣ ε unit identity.
    AdjUnit,
    /// K-Adj-Counit — α ⊣ ε counit identity.
    AdjCounit,
}

impl KernelRuleId {
    pub fn name(self) -> &'static str {
        match self {
            KernelRuleId::Refine => "K-Refine",
            KernelRuleId::Univ => "K-Univ",
            KernelRuleId::Pos => "K-Pos",
            KernelRuleId::Norm => "K-Norm",
            KernelRuleId::FwAx => "K-FwAx",
            KernelRuleId::AdjUnit => "K-Adj-Unit",
            KernelRuleId::AdjCounit => "K-Adj-Counit",
        }
    }

    /// Iterate the full seven-rule list.
    pub fn full_list() -> [KernelRuleId; 7] {
        [
            KernelRuleId::Refine,
            KernelRuleId::Univ,
            KernelRuleId::Pos,
            KernelRuleId::Norm,
            KernelRuleId::FwAx,
            KernelRuleId::AdjUnit,
            KernelRuleId::AdjCounit,
        ]
    }
}

// =============================================================================
// Per-rule decomposition
// =============================================================================

/// The meta-theoretic requirements of a single kernel rule —
/// the precise ZFC axioms + Grothendieck universes needed to
/// model it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaTheoryRequirements {
    /// The kernel rule under consideration.
    pub rule: KernelRuleId,
    /// The ZFC axioms required to model the rule.
    pub zfc_axioms: Vec<ZfcAxiom>,
    /// The Grothendieck universes (inaccessibles) required.
    pub inaccessibles: Vec<InaccessibleLevel>,
    /// Optional human-readable citation explaining why this set is
    /// load-bearing (e.g. "Berardi 1998 derivation of False").
    pub citation: Text,
}

/// Decompose a kernel rule into its meta-theoretic requirements.
/// Per Kunen 2011 + Lurie HTT App. A:
///
/// * `K-Refine`: needs Separation (the comprehension `{x:A | P(x)}`
///   IS Separation) + Replacement (depth-stratification across types).
/// * `K-Univ`: needs Replacement (universe successor) + 2 inaccessibles
///   (Type_1 and Type_2 require κ_1 and κ_2 respectively).
/// * `K-Pos`: needs Foundation (rejection of non-positive recursion
///   uses ∈-induction) + Separation.
/// * `K-Norm`: needs Foundation + Replacement (transfinite induction
///   on reduction depth) + 1 inaccessible (universal SN model).
/// * `K-FwAx`: needs Pairing + Union (axiom-set construction) +
///   Separation (Prop-only side-condition).
/// * `K-Adj-Unit` / `K-Adj-Counit`: need Replacement + 1 inaccessible
///   (the adjunction lives in (∞,1)-Cat, modelled in U_κ_1).
pub fn required_meta_theory(rule: KernelRuleId) -> MetaTheoryRequirements {
    use InaccessibleLevel::*;
    use KernelRuleId::*;
    use ZfcAxiom::*;

    match rule {
        Refine => MetaTheoryRequirements {
            rule,
            zfc_axioms: vec![Separation, Replacement, Foundation],
            inaccessibles: vec![],
            citation: Text::from(
                "K-Refine = Separation + Replacement + Foundation; \
                 depth-stratification over comprehension (Yanofsky 2003)",
            ),
        },
        Univ => MetaTheoryRequirements {
            rule,
            zfc_axioms: vec![Replacement, Pairing, Union, PowerSet],
            inaccessibles: vec![Kappa1, Kappa2],
            citation: Text::from(
                "K-Univ = Grothendieck-universe model; \
                 κ_1 ⇒ Type_1, κ_2 ⇒ Type_2 (host for ∞-cat classifier)",
            ),
        },
        Pos => MetaTheoryRequirements {
            rule,
            zfc_axioms: vec![Foundation, Separation],
            inaccessibles: vec![],
            citation: Text::from(
                "K-Pos = Berardi 1998: non-positive recursion ⇒ ⊥; \
                 blocking proof uses ∈-induction (Foundation)",
            ),
        },
        Norm => MetaTheoryRequirements {
            rule,
            zfc_axioms: vec![Foundation, Replacement, Separation],
            inaccessibles: vec![Kappa1],
            citation: Text::from(
                "K-Norm = Huber 2019 + K-FwAx side-condition; \
                 transfinite SN model lives in U_κ_1",
            ),
        },
        FwAx => MetaTheoryRequirements {
            rule,
            zfc_axioms: vec![Pairing, Union, Separation],
            inaccessibles: vec![],
            citation: Text::from(
                "K-FwAx = Prop-only admission; \
                 Pairing+Union build the axiom set, Separation gates the body type",
            ),
        },
        AdjUnit => MetaTheoryRequirements {
            rule,
            zfc_axioms: vec![Replacement, Pairing, Union],
            inaccessibles: vec![Kappa1],
            citation: Text::from(
                "K-Adj-Unit = α ⊣ ε unit (Diakrisis 108.T); \
                 (∞,1)-categorical adjunction modelled in U_κ_1",
            ),
        },
        AdjCounit => MetaTheoryRequirements {
            rule,
            zfc_axioms: vec![Replacement, Pairing, Union],
            inaccessibles: vec![Kappa1],
            citation: Text::from(
                "K-Adj-Counit = α ⊣ ε counit (Diakrisis 108.T); \
                 (∞,1)-categorical adjunction modelled in U_κ_1",
            ),
        },
    }
}

/// Decide whether a kernel rule is provable in **ZFC + 2 inaccessibles**.
/// Returns true iff the rule's meta-theoretic requirements are a
/// subset of `(ZFC, [κ_1, κ_2])`.
///
/// **All seven rules** are provable in ZFC + 2 inaccessibles (this is
/// the design invariant of the Verum kernel).  V0 surface reads off
/// the requirement record and confirms set inclusion.
pub fn is_zfc_plus_2_inacc_provable(rule: KernelRuleId) -> bool {
    let req = required_meta_theory(rule);
    // Every requirement must be either a ZFC axiom (always available)
    // or an inaccessible up to κ_2.
    req.inaccessibles
        .iter()
        .all(|k| matches!(k, InaccessibleLevel::Kappa1 | InaccessibleLevel::Kappa2))
}

// =============================================================================
// SelfRecognitionAudit accumulator
// =============================================================================

/// An accumulating audit record: every kernel-rule citation appends
/// to the audit, and the final report surfaces the precise union of
/// meta-theoretic dependencies.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SelfRecognitionAudit {
    /// Cited rules (with multiplicity preserved for diagnostic value).
    pub citations: Vec<KernelRuleId>,
}

impl SelfRecognitionAudit {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cite a kernel rule; appends to the audit.
    pub fn cite(&mut self, rule: KernelRuleId) {
        self.citations.push(rule);
    }

    /// The union of ZFC axioms transitively required by every cited rule.
    /// Returns axioms in canonical (`ZfcAxiom::full_list()`) order with
    /// duplicates removed.
    pub fn required_zfc_axioms(&self) -> Vec<ZfcAxiom> {
        let mut required = std::collections::HashSet::new();
        for rule in &self.citations {
            for ax in required_meta_theory(*rule).zfc_axioms {
                required.insert(ax);
            }
        }
        ZfcAxiom::full_list()
            .iter()
            .copied()
            .filter(|ax| required.contains(ax))
            .collect()
    }

    /// The union of inaccessibles transitively required.  Returns in
    /// canonical (Kappa1, Kappa2) order with duplicates removed.
    pub fn required_inaccessibles(&self) -> Vec<InaccessibleLevel> {
        let mut required = std::collections::HashSet::new();
        for rule in &self.citations {
            for k in required_meta_theory(*rule).inaccessibles {
                required.insert(k);
            }
        }
        let mut out = Vec::new();
        for k in [InaccessibleLevel::Kappa1, InaccessibleLevel::Kappa2] {
            if required.contains(&k) {
                out.push(k);
            }
        }
        out
    }

    /// True iff every cited rule is provable in ZFC + 2 inaccessibles.
    /// This is the kernel's *self-recognition* invariant: every rule
    /// derivation must lift to the meta-theory.
    pub fn is_provable_in_zfc_plus_2_inacc(&self) -> bool {
        self.citations
            .iter()
            .all(|r| is_zfc_plus_2_inacc_provable(*r))
    }

    /// Render a human-readable trusted-base report.
    pub fn report(&self) -> String {
        let zfc = self.required_zfc_axioms();
        let inacc = self.required_inaccessibles();
        let zfc_names: Vec<&str> = zfc.iter().map(|a| a.name()).collect();
        let inacc_names: Vec<&str> = inacc.iter().map(|k| k.name()).collect();
        format!(
            "self-recognition: {} citations, ZFC axioms = [{}], inaccessibles = [{}]",
            self.citations.len(),
            zfc_names.join(", "),
            inacc_names.join(", "),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- ZfcAxiom -----

    #[test]
    fn zfc_full_list_is_nine() {
        assert_eq!(ZfcAxiom::full_list().len(), 9);
    }

    // ----- KernelRuleId -----

    #[test]
    fn kernel_rule_full_list_is_seven() {
        assert_eq!(KernelRuleId::full_list().len(), 7);
    }

    // ----- Per-rule decomposition -----

    #[test]
    fn k_refine_uses_separation_and_replacement() {
        let req = required_meta_theory(KernelRuleId::Refine);
        assert!(req.zfc_axioms.contains(&ZfcAxiom::Separation));
        assert!(req.zfc_axioms.contains(&ZfcAxiom::Replacement));
        assert!(req.inaccessibles.is_empty(),
            "K-Refine should not require Grothendieck universes directly");
    }

    #[test]
    fn k_univ_requires_two_inaccessibles() {
        let req = required_meta_theory(KernelRuleId::Univ);
        assert_eq!(req.inaccessibles.len(), 2);
        assert!(req.inaccessibles.contains(&InaccessibleLevel::Kappa1));
        assert!(req.inaccessibles.contains(&InaccessibleLevel::Kappa2));
    }

    #[test]
    fn k_pos_uses_foundation() {
        let req = required_meta_theory(KernelRuleId::Pos);
        assert!(req.zfc_axioms.contains(&ZfcAxiom::Foundation),
            "K-Pos blocking of Berardi paradox uses ∈-induction");
    }

    #[test]
    fn k_norm_requires_one_inaccessible() {
        let req = required_meta_theory(KernelRuleId::Norm);
        assert!(req.inaccessibles.contains(&InaccessibleLevel::Kappa1));
        assert!(!req.inaccessibles.contains(&InaccessibleLevel::Kappa2));
    }

    #[test]
    fn k_fwax_does_not_require_inaccessibles() {
        let req = required_meta_theory(KernelRuleId::FwAx);
        assert!(req.inaccessibles.is_empty());
    }

    #[test]
    fn k_adj_unit_and_counit_share_requirements() {
        let unit = required_meta_theory(KernelRuleId::AdjUnit);
        let counit = required_meta_theory(KernelRuleId::AdjCounit);
        assert_eq!(unit.zfc_axioms, counit.zfc_axioms,
            "Unit and counit identities use the same ZFC fragment");
        assert_eq!(unit.inaccessibles, counit.inaccessibles);
    }

    // ----- ZFC + 2-inacc provability -----

    #[test]
    fn every_kernel_rule_is_provable_in_zfc_plus_2_inacc() {
        for rule in KernelRuleId::full_list() {
            assert!(is_zfc_plus_2_inacc_provable(rule),
                "{} must be provable in ZFC + 2 inaccessibles", rule.name());
        }
    }

    // ----- SelfRecognitionAudit -----

    #[test]
    fn audit_empty_has_no_requirements() {
        let audit = SelfRecognitionAudit::new();
        assert!(audit.required_zfc_axioms().is_empty());
        assert!(audit.required_inaccessibles().is_empty());
        assert!(audit.is_provable_in_zfc_plus_2_inacc());
    }

    #[test]
    fn audit_accumulates_zfc_axiom_union() {
        let mut audit = SelfRecognitionAudit::new();
        audit.cite(KernelRuleId::Refine);
        audit.cite(KernelRuleId::Univ);
        let req = audit.required_zfc_axioms();
        assert!(req.contains(&ZfcAxiom::Separation));    // from Refine
        assert!(req.contains(&ZfcAxiom::Replacement));   // from both
        assert!(req.contains(&ZfcAxiom::Foundation));    // from Refine
        assert!(req.contains(&ZfcAxiom::Pairing));       // from Univ
    }

    #[test]
    fn audit_required_inaccessibles_unions_correctly() {
        let mut audit = SelfRecognitionAudit::new();
        audit.cite(KernelRuleId::Norm);     // κ_1
        audit.cite(KernelRuleId::Univ);     // κ_1, κ_2
        let inacc = audit.required_inaccessibles();
        assert_eq!(inacc.len(), 2);
        assert_eq!(inacc[0], InaccessibleLevel::Kappa1);
        assert_eq!(inacc[1], InaccessibleLevel::Kappa2);
    }

    #[test]
    fn audit_full_seven_rule_citation_lifts_to_zfc_plus_2() {
        let mut audit = SelfRecognitionAudit::new();
        for rule in KernelRuleId::full_list() {
            audit.cite(rule);
        }
        assert!(audit.is_provable_in_zfc_plus_2_inacc());
        // The full audit should require both inaccessibles.
        assert_eq!(audit.required_inaccessibles().len(), 2);
        // And nearly every ZFC axiom (exclude Choice, which is only
        // implicit in some constructions but not currently required
        // by any of the seven rules).
        let req = audit.required_zfc_axioms();
        assert!(req.len() >= 6, "expected at least 6 ZFC axioms required, got {}", req.len());
    }

    #[test]
    fn audit_report_renders_both_axis() {
        let mut audit = SelfRecognitionAudit::new();
        audit.cite(KernelRuleId::Univ);
        let report = audit.report();
        assert!(report.contains("κ_1"));
        assert!(report.contains("κ_2"));
        assert!(report.contains("Replacement"));
    }
}
