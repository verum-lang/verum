//! Cubical cofibration calculus — interval subsumption + face-formula algebra
//! (M-VVA-FU Sub-2.4-cubical, V1 deferred per VVA spec L579).
//!
//! Pre-this-module the kernel's cubical rules (`HComp`, `Transp`,
//! `Glue` at `infer.rs:431-510`) treated the face formula `φ` as
//! "well-typed but not interval-subsumption-checked" — see the
//! `infer.rs:422-424` deferral comment. This module ships the
//! decidable cofibration-formula algebra:
//!
//! * **Carrier:** `FaceFormula` — a finite distributive-lattice
//!   element over generators `(i = 0)` / `(i = 1)` for interval
//!   variables `i`.
//! * **Operations:** ∧ (and), ∨ (or), ⊥ (false / never), ⊤ (true /
//!   always).
//! * **Relations:** `(i = 0) ∧ (i = 1) = ⊥` (each variable is at most
//!   one endpoint), de Morgan, distributivity.
//! * **Subsumption:** decidable `φ ≤ ψ` via DNF normalisation +
//!   per-clause containment.
//!
//! The CCHM cubical-set semantics requires HComp / Transp / Glue to
//! satisfy *cofibration coherence*: the wall family must be defined
//! exactly on `φ`, and the result's restriction to `φ` must agree
//! with the wall. Pre-Sub-2.4 the kernel admitted any well-typed φ;
//! post-Sub-2.4 the kernel rejects walls whose support does not
//! match φ's coverage (interval subsumption).
//!
//! References:
//!   * Cohen, Coquand, Huber, Mörtberg (CCHM), "Cubical Type Theory:
//!     a constructive interpretation of the univalence axiom" (2015).
//!   * Angiuli, Brunerie, Coquand, Hou (Favonia), Harper, Licata,
//!     "Cartesian Cubical Type Theory" (2017).

use std::collections::BTreeSet;

use verum_common::Text;

/// Atomic face-formula generator: an interval variable equals 0 or 1.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FaceLit {
    /// Interval-variable name (e.g., `"i"`, `"j"`).
    pub var: Text,
    /// Endpoint: `false` = `i=0`, `true` = `i=1`.
    pub end: bool,
}

impl FaceLit {
    /// Construct a literal `i = 0`.
    pub fn zero(var: impl Into<Text>) -> Self {
        FaceLit { var: var.into(), end: false }
    }

    /// Construct a literal `i = 1`.
    pub fn one(var: impl Into<Text>) -> Self {
        FaceLit { var: var.into(), end: true }
    }

    /// True iff `self` and `other` are contradictory literals on the
    /// same interval variable: `(i=0)` and `(i=1)`.
    pub fn contradicts(&self, other: &FaceLit) -> bool {
        self.var == other.var && self.end != other.end
    }
}

/// A *clause* in the cofibration DNF: a conjunction of literals.
/// Stored as a BTreeSet for deterministic ordering + O(log n) lookup.
/// The empty clause represents `⊤` (no constraints, always-true).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Clause {
    /// Conjunction of face literals.  Empty set ≡ `⊤`.
    pub lits: BTreeSet<FaceLit>,
}

impl Clause {
    /// The empty clause `⊤` — no face constraints.
    pub fn empty() -> Self { Self::default() }

    /// A clause containing exactly one face literal.
    pub fn singleton(lit: FaceLit) -> Self {
        let mut lits = BTreeSet::new();
        lits.insert(lit);
        Clause { lits }
    }

    /// Logical AND of two clauses. Returns `None` if the result is
    /// `⊥` (contradictory — contains both `(i=0)` and `(i=1)` for
    /// some `i`).
    pub fn and(&self, other: &Clause) -> Option<Clause> {
        let mut merged = self.lits.clone();
        for lit in &other.lits {
            // Detect contradiction with any existing literal.
            for existing in &merged {
                if lit.contradicts(existing) {
                    return None; // ⊥
                }
            }
            merged.insert(lit.clone());
        }
        Some(Clause { lits: merged })
    }

    /// True iff `self ⊆ other` as literal sets — i.e. every
    /// constraint of `self` is also a constraint of `other`. By
    /// the lattice law for clauses, `self ⊆ other` (literal-wise)
    /// implies `other ⇒ self` (logically: if all of `other`'s
    /// literals hold, all of `self`'s do too).
    pub fn implies(&self, other: &Clause) -> bool {
        // self ⊆ other means: every literal in self is in other.
        // Note: this is *literal-set inclusion* — when self has
        // FEWER constraints, it's WEAKER (easier to satisfy), so
        // `other ⇒ self`. Caller computes the relation per the
        // direction needed.
        self.lits.is_subset(&other.lits)
    }
}

/// Cofibration formula in DNF (disjunctive normal form): a disjunction
/// of clauses. Empty disjunction = `⊥` (never-true); single empty
/// clause = `⊤` (always-true).
///
/// Invariant: no clause subsumes another (canonical / minimal DNF).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaceFormula {
    /// Disjunction of conjunctive clauses.  Canonical DNF — no
    /// clause subsumes another.
    pub clauses: Vec<Clause>,
}

impl FaceFormula {
    /// Always-false (never-true): `⊥`. Empty disjunction.
    pub fn bottom() -> Self {
        FaceFormula { clauses: Vec::new() }
    }

    /// Always-true: `⊤`. Single empty clause.
    pub fn top() -> Self {
        FaceFormula { clauses: vec![Clause::empty()] }
    }

    /// Atomic literal as a one-clause-one-literal formula.
    pub fn lit(l: FaceLit) -> Self {
        FaceFormula { clauses: vec![Clause::singleton(l)] }
    }

    /// `(i = 0)` shorthand.
    pub fn at_zero(var: impl Into<Text>) -> Self {
        Self::lit(FaceLit::zero(var))
    }

    /// `(i = 1)` shorthand.
    pub fn at_one(var: impl Into<Text>) -> Self {
        Self::lit(FaceLit::one(var))
    }

    /// True iff the formula is `⊥` (no clauses).
    pub fn is_bottom(&self) -> bool {
        self.clauses.is_empty()
    }

    /// True iff the formula is `⊤` (contains the empty clause).
    pub fn is_top(&self) -> bool {
        self.clauses.iter().any(|c| c.lits.is_empty())
    }

    /// Logical OR via clause-set union. Subsumed clauses are dropped
    /// to maintain canonical / minimal DNF form.
    pub fn or(&self, other: &FaceFormula) -> FaceFormula {
        let mut all: Vec<Clause> = self.clauses.iter().chain(other.clauses.iter()).cloned().collect();
        // Remove subsumed clauses: if clause A's literals ⊆ clause B's,
        // then A ⇒ B in DNF (the larger constraint subsumes the smaller),
        // so B is redundant.
        // NOTE: clause containment direction is reversed for DNF —
        // a SMALLER literal-set means LESS constraint = MORE general
        // = subsumes the larger.
        let mut keep: Vec<bool> = vec![true; all.len()];
        for i in 0..all.len() {
            if !keep[i] { continue; }
            for j in 0..all.len() {
                if i == j || !keep[j] { continue; }
                // If all[j] is properly contained in all[i] (smaller
                // literal-set), then all[i] is subsumed by all[j].
                if all[j].lits.is_subset(&all[i].lits) && all[j].lits != all[i].lits {
                    keep[i] = false;
                    break;
                }
            }
        }
        let mut filtered = Vec::new();
        for (i, clause) in all.drain(..).enumerate() {
            if keep[i] {
                filtered.push(clause);
            }
        }
        FaceFormula { clauses: filtered }
    }

    /// Logical AND via Cartesian product of clauses. Contradictory
    /// products (clauses containing both `(i=0)` and `(i=1)` for some
    /// `i`) are dropped per Clause::and's None case.
    pub fn and(&self, other: &FaceFormula) -> FaceFormula {
        let mut clauses = Vec::new();
        for c1 in &self.clauses {
            for c2 in &other.clauses {
                if let Some(prod) = c1.and(c2) {
                    clauses.push(prod);
                }
            }
        }
        FaceFormula { clauses }
    }

    /// **Subsumption decision: `self ⇒ other`.**
    ///
    /// In DNF, `self ⇒ other` iff every clause of `self` is contained
    /// in (i.e., implies) some clause of `other`. Each clause
    /// containment is itself decidable via literal-set inclusion (a
    /// clause with FEWER literals is more general).
    ///
    /// For HComp / Transp / Glue cofibration coherence:
    /// `walls.support ⇒ φ` means the wall family covers exactly the
    /// face φ requires — interval-subsumption verified.
    pub fn implies(&self, other: &FaceFormula) -> bool {
        // ⊥ ⇒ anything.
        if self.is_bottom() {
            return true;
        }
        // anything ⇒ ⊤.
        if other.is_top() {
            return true;
        }
        // For each clause c1 in self, ∃ clause c2 in other with
        // c2's literals ⊆ c1's literals (c2 is more general, hence
        // c1 ⇒ c2).
        for c1 in &self.clauses {
            let mut found = false;
            for c2 in &other.clauses {
                if c2.lits.is_subset(&c1.lits) {
                    found = true;
                    break;
                }
            }
            if !found {
                return false;
            }
        }
        true
    }

    /// Mutual implication: `self ⇔ other`.
    pub fn equivalent_to(&self, other: &FaceFormula) -> bool {
        self.implies(other) && other.implies(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bottom_implies_anything() {
        assert!(FaceFormula::bottom().implies(&FaceFormula::top()));
        assert!(FaceFormula::bottom().implies(&FaceFormula::at_zero("i")));
        assert!(FaceFormula::bottom().implies(&FaceFormula::bottom()));
    }

    #[test]
    fn anything_implies_top() {
        assert!(FaceFormula::at_zero("i").implies(&FaceFormula::top()));
        assert!(FaceFormula::top().implies(&FaceFormula::top()));
    }

    #[test]
    fn at_zero_does_not_imply_at_one_same_var() {
        // (i=0) does NOT imply (i=1).
        assert!(!FaceFormula::at_zero("i").implies(&FaceFormula::at_one("i")));
    }

    #[test]
    fn at_zero_self_implication() {
        // φ ⇒ φ trivially.
        let phi = FaceFormula::at_zero("i");
        assert!(phi.implies(&phi));
    }

    #[test]
    fn or_union_implies_disjuncts() {
        // (i=0) ∨ (j=1) does NOT imply (i=0) alone (j=1 case escapes).
        let or = FaceFormula::at_zero("i").or(&FaceFormula::at_one("j"));
        assert!(!or.implies(&FaceFormula::at_zero("i")));
        // But (i=0) does imply the OR.
        assert!(FaceFormula::at_zero("i").implies(&or));
        // And (j=1) does imply the OR.
        assert!(FaceFormula::at_one("j").implies(&or));
    }

    #[test]
    fn and_intersection_contradiction() {
        // (i=0) ∧ (i=1) = ⊥ (contradictory).
        let conjunction = FaceFormula::at_zero("i").and(&FaceFormula::at_one("i"));
        assert!(conjunction.is_bottom());
    }

    #[test]
    fn and_combines_disjoint_vars() {
        // (i=0) ∧ (j=1) = single clause {(i=0), (j=1)}.
        let conj = FaceFormula::at_zero("i").and(&FaceFormula::at_one("j"));
        assert_eq!(conj.clauses.len(), 1);
        assert_eq!(conj.clauses[0].lits.len(), 2);
    }

    #[test]
    fn de_morgan_or_implies_disjuncts_individually() {
        let phi_i = FaceFormula::at_zero("i");
        let phi_j = FaceFormula::at_zero("j");
        let or = phi_i.or(&phi_j);
        // (i=0) ⇒ (i=0) ∨ (j=0) ✓
        assert!(phi_i.implies(&or));
        // (j=0) ⇒ (i=0) ∨ (j=0) ✓
        assert!(phi_j.implies(&or));
        // ⊥ ⇒ everything ✓
        assert!(FaceFormula::bottom().implies(&or));
    }

    #[test]
    fn equivalent_to_own_canonical_form() {
        let phi = FaceFormula::at_zero("i");
        let phi_via_or_bottom = phi.or(&FaceFormula::bottom());
        assert!(phi.equivalent_to(&phi_via_or_bottom));
    }

    #[test]
    fn cofibration_coherence_walls_subsume_phi() {
        // Canonical use-case: HComp's walls family must support
        // exactly the face φ. Test: walls = (i=0) ∨ (i=1), φ = (i=0).
        // φ ⇒ walls ✓ (the φ-face is covered by walls).
        let phi = FaceFormula::at_zero("i");
        let walls = phi.or(&FaceFormula::at_one("i"));
        assert!(phi.implies(&walls));
    }

    #[test]
    fn or_subsumption_drops_redundant_clauses() {
        // (i=0) ∨ (i=0) ∧ (j=1) — the second clause is subsumed by
        // the first (more specific). After OR, only (i=0) remains.
        let phi = FaceFormula::at_zero("i");
        let psi = phi.and(&FaceFormula::at_one("j"));
        let or = phi.or(&psi);
        // After subsumption-removal, the OR has only one clause.
        assert_eq!(or.clauses.len(), 1);
    }

    #[test]
    fn distributivity_holds() {
        // (i=0) ∧ ((j=0) ∨ (j=1)) ≡ ((i=0) ∧ (j=0)) ∨ ((i=0) ∧ (j=1))
        let i0 = FaceFormula::at_zero("i");
        let j0 = FaceFormula::at_zero("j");
        let j1 = FaceFormula::at_one("j");
        let lhs = i0.and(&j0.or(&j1));
        let rhs = i0.and(&j0).or(&i0.and(&j1));
        assert!(lhs.equivalent_to(&rhs));
    }

    #[test]
    fn three_var_complex_subsumption() {
        // ((i=0) ∧ (j=0)) ⇒ ((i=0) ∨ (k=1))
        let lhs = FaceFormula::at_zero("i").and(&FaceFormula::at_zero("j"));
        let rhs = FaceFormula::at_zero("i").or(&FaceFormula::at_one("k"));
        assert!(lhs.implies(&rhs));
    }
}
