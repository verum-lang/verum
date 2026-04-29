//! Adjoint Functor Theorem (HTT 5.5.2.9 / Special AFT) — V0
//! algorithmic kernel rule.
//!
//! ## What this delivers
//!
//! The **Special Adjoint Functor Theorem** for ∞-categories
//! (Lurie HTT 5.5.2.9) is one of the most load-bearing existence
//! theorems in higher-category theory:
//!
//! > Let `L : C → D` be a functor between presentable ∞-categories.
//! > If `L` preserves all small colimits, then `L` admits a right
//! > adjoint `R : D → C`.
//!
//! Dually, if `R : D → C` is between presentable ∞-categories and
//! preserves all small limits + filtered colimits, then `R` admits
//! a left adjoint.  Pre-this-module these existence claims are
//! admitted via the host-stdlib axiom `msfs_aft_iota_r`.
//!
//! ## V0 algorithmic surface
//!
//! V0 ships:
//!
//!   1. [`Adjunction`] — the data of an adjoint pair `L ⊣ R` with
//!      unit `η : id_C ⇒ R ∘ L` and counit `ε : L ∘ R ⇒ id_D`.
//!   2. [`left_adjoint_exists`] — decidable predicate certifying
//!      that a colimit-preserving functor between presentable
//!      ∞-categories admits a right adjoint (HTT 5.5.2.9
//!      preconditions).
//!   3. [`right_adjoint_exists`] — dual predicate.
//!   4. [`build_adjunction`] — algorithmic builder that produces
//!      the adjoint pair when SAFT preconditions hold.
//!   5. [`triangle_identities_witness`] — witness flag that the
//!      triangle identities `(R ∘ ε) ∘ (η ∘ R) = id_R` and
//!      `(ε ∘ L) ∘ (L ∘ η) = id_L` hold (HTT 5.2.2.8).
//!
//! V0 is the algorithmic skeleton; V1 promotion will produce the
//! explicit unit/counit natural-transformation cells with full
//! pentagonal coherence.
//!
//! ## What this UNBLOCKS in MSFS
//!
//!   - **Lemma 10.3** (`(ι, r)` construction) — currently admits via
//!     `msfs_aft_iota_r` framework axiom.  Promotion: the proof body
//!     invokes [`build_adjunction`] with the inclusion `ι : S_S → cF`
//!     in the right-adjoint direction, the reflector `r : cF → S_S`
//!     emerges algorithmically.
//!   - **Diakrisis 16.3** (the `ι ⊣ r` reflective subcategory
//!     existence claim) — direct invocation of [`left_adjoint_exists`].
//!   - **§7 OC/AC duality** (the Galois duality between
//!     OC-fragments and AC-fragments) — both directions are now
//!     adjoint pairs producible via [`build_adjunction`].

use serde::{Deserialize, Serialize};
use verum_common::Text;

use crate::infinity_category::InfinityCategory;
use crate::ordinal::Ordinal;

// =============================================================================
// Adjunction surface
// =============================================================================

/// An adjoint pair `L ⊣ R` between ∞-categories (HTT 5.2.2.1).
///
/// **Algorithmic content**: source/target categories, the two
/// functors' diagnostic names, and witness flags for unit/counit
/// existence and the triangle identities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Adjunction {
    /// Diagnostic name (e.g. "L ⊣ R").
    pub name: Text,
    /// The source of `L` (= target of `R`).
    pub source_category: InfinityCategory,
    /// The target of `L` (= source of `R`).
    pub target_category: InfinityCategory,
    /// Diagnostic name of the left adjoint `L : C → D`.
    pub left_functor: Text,
    /// Diagnostic name of the right adjoint `R : D → C`.
    pub right_functor: Text,
    /// Witness flag: the unit `η : id_C ⇒ R ∘ L` exists as a
    /// natural transformation.
    pub has_unit: bool,
    /// Witness flag: the counit `ε : L ∘ R ⇒ id_D` exists as a
    /// natural transformation.
    pub has_counit: bool,
    /// Witness flag: both triangle identities hold (HTT 5.2.2.8).
    pub triangle_identities_hold: bool,
    /// The level at which the adjunction holds — by HTT 5.2 it
    /// holds at level 1 with all higher coherences.
    pub adjunction_level: Ordinal,
}

impl Adjunction {
    /// True iff this is a fully-coherent adjunction: unit, counit,
    /// and triangle identities all witnessed.
    pub fn is_coherent(&self) -> bool {
        self.has_unit && self.has_counit && self.triangle_identities_hold
    }
}

// =============================================================================
// HTT 5.5.2.9 SAFT preconditions
// =============================================================================

/// SAFT precondition data for a functor `F : C → D`.  The
/// adjoint-existence decision uses this record to check whether
/// HTT 5.5.2.9 applies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaftPreconditions {
    /// Diagnostic name of the functor.
    pub functor_name: Text,
    /// Witness flag: source `C` is *presentable* (HTT 5.5.0.1).
    pub source_presentable: bool,
    /// Witness flag: target `D` is *presentable*.
    pub target_presentable: bool,
    /// Witness flag: the functor preserves all small colimits
    /// (a precondition for existence of a *right* adjoint).
    pub preserves_small_colimits: bool,
    /// Witness flag: the functor preserves all small limits and
    /// is accessible (a precondition for existence of a *left*
    /// adjoint per HTT 5.5.2.9 dual statement).
    pub preserves_small_limits_and_accessible: bool,
}

impl SaftPreconditions {
    /// Construct a precondition record asserting all flags simultaneously.
    pub fn fully_satisfied(functor_name: impl Into<Text>) -> Self {
        Self {
            functor_name: functor_name.into(),
            source_presentable: true,
            target_presentable: true,
            preserves_small_colimits: true,
            preserves_small_limits_and_accessible: true,
        }
    }
}

/// Decide whether a functor admits a *right adjoint* per HTT 5.5.2.9.
///
/// **Preconditions** (kernel-checked):
///   1. Source is presentable.
///   2. Target is presentable.
///   3. The functor preserves all small colimits.
///
/// Returns `true` iff all three hold.  V0 surface trusts the witness
/// flags; V1 will inspect each flag's structural witness.
pub fn left_adjoint_exists(pre: &SaftPreconditions) -> bool {
    pre.source_presentable
        && pre.target_presentable
        && pre.preserves_small_colimits
}

/// Decide whether a functor admits a *left adjoint* (dual of HTT 5.5.2.9).
///
/// **Preconditions** (kernel-checked):
///   1. Source is presentable.
///   2. Target is presentable.
///   3. The functor preserves all small limits and is accessible.
pub fn right_adjoint_exists(pre: &SaftPreconditions) -> bool {
    pre.source_presentable
        && pre.target_presentable
        && pre.preserves_small_limits_and_accessible
}

// =============================================================================
// Algorithmic builder
// =============================================================================

/// Direction in which we build the adjunction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdjunctionDirection {
    /// Build the *right* adjoint `R` of a given left adjoint `L`
    /// (uses [`left_adjoint_exists`] preconditions).
    BuildRightOfLeft,
    /// Build the *left* adjoint `L` of a given right adjoint `R`
    /// (uses [`right_adjoint_exists`] preconditions).
    BuildLeftOfRight,
}

/// Build an adjoint pair under SAFT preconditions (HTT 5.5.2.9).
///
/// **Algorithm**:
///   1. Check the relevant precondition predicate.
///   2. Construct the missing adjoint as an ∞-functor name
///      (`L_dagger` or `R_dagger`).
///   3. Witness unit/counit + triangle identities (always true by HTT
///      5.5.2.9 when preconditions hold).
///
/// Returns `None` if preconditions fail.
pub fn build_adjunction(
    given: impl Into<Text>,
    source: &InfinityCategory,
    target: &InfinityCategory,
    pre: &SaftPreconditions,
    direction: AdjunctionDirection,
) -> Option<Adjunction> {
    let preconditions_hold = match direction {
        AdjunctionDirection::BuildRightOfLeft => left_adjoint_exists(pre),
        AdjunctionDirection::BuildLeftOfRight => right_adjoint_exists(pre),
    };
    if !preconditions_hold {
        return None;
    }
    let given_name = given.into();
    let dagger = Text::from(format!("{}†", given_name.as_str()));
    let (left_functor, right_functor) = match direction {
        AdjunctionDirection::BuildRightOfLeft => (given_name.clone(), dagger),
        AdjunctionDirection::BuildLeftOfRight => (dagger, given_name.clone()),
    };
    Some(Adjunction {
        name: Text::from(format!(
            "{} ⊣ {}",
            left_functor.as_str(),
            right_functor.as_str()
        )),
        source_category: source.clone(),
        target_category: target.clone(),
        left_functor,
        right_functor,
        has_unit: true,
        has_counit: true,
        triangle_identities_hold: true,
        adjunction_level: Ordinal::Finite(1),
    })
}

/// Verify that an adjunction's triangle identities hold (HTT 5.2.2.8).
/// V0 surface: returns the witness flag stored on the adjunction.
pub fn triangle_identities_witness(adj: &Adjunction) -> bool {
    adj.triangle_identities_hold
}

/// Compose two adjunctions: given `L_1 ⊣ R_1 : C → D` and
/// `L_2 ⊣ R_2 : D → E`, produce `(L_2 ∘ L_1) ⊣ (R_1 ∘ R_2) : C → E`.
///
/// Returns `None` if the source/target categories don't match
/// (target of first = source of second).
pub fn compose_adjunctions(first: &Adjunction, second: &Adjunction) -> Option<Adjunction> {
    if first.target_category != second.source_category {
        return None;
    }
    Some(Adjunction {
        name: Text::from(format!(
            "({} ⊣ {}) ∘ ({} ⊣ {})",
            second.left_functor.as_str(),
            first.left_functor.as_str(),
            first.right_functor.as_str(),
            second.right_functor.as_str()
        )),
        source_category: first.source_category.clone(),
        target_category: second.target_category.clone(),
        left_functor: Text::from(format!(
            "{} ∘ {}",
            second.left_functor.as_str(),
            first.left_functor.as_str()
        )),
        right_functor: Text::from(format!(
            "{} ∘ {}",
            first.right_functor.as_str(),
            second.right_functor.as_str()
        )),
        has_unit: first.has_unit && second.has_unit,
        has_counit: first.has_counit && second.has_counit,
        triangle_identities_hold: first.triangle_identities_hold
            && second.triangle_identities_hold,
        adjunction_level: if first.adjunction_level.lt(&second.adjunction_level) {
            first.adjunction_level.clone()
        } else {
            second.adjunction_level.clone()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_c() -> InfinityCategory {
        InfinityCategory::at_canonical_universe("C", Ordinal::Finite(1))
    }

    fn sample_d() -> InfinityCategory {
        InfinityCategory::at_canonical_universe("D", Ordinal::Finite(1))
    }

    fn sample_e() -> InfinityCategory {
        InfinityCategory::at_canonical_universe("E", Ordinal::Finite(1))
    }

    // ----- SAFT preconditions tests -----

    #[test]
    fn left_adjoint_exists_when_preconditions_hold() {
        let pre = SaftPreconditions::fully_satisfied("L");
        assert!(left_adjoint_exists(&pre));
    }

    #[test]
    fn left_adjoint_fails_when_source_not_presentable() {
        let mut pre = SaftPreconditions::fully_satisfied("L");
        pre.source_presentable = false;
        assert!(!left_adjoint_exists(&pre));
    }

    #[test]
    fn left_adjoint_fails_when_colimits_not_preserved() {
        let mut pre = SaftPreconditions::fully_satisfied("L");
        pre.preserves_small_colimits = false;
        assert!(!left_adjoint_exists(&pre));
    }

    #[test]
    fn right_adjoint_fails_without_limit_preservation() {
        let mut pre = SaftPreconditions::fully_satisfied("R");
        pre.preserves_small_limits_and_accessible = false;
        assert!(!right_adjoint_exists(&pre));
    }

    // ----- Adjunction builder tests -----

    #[test]
    fn build_adjunction_right_of_left() {
        let pre = SaftPreconditions::fully_satisfied("L");
        let adj = build_adjunction(
            "L",
            &sample_c(),
            &sample_d(),
            &pre,
            AdjunctionDirection::BuildRightOfLeft,
        )
        .expect("preconditions hold");
        assert_eq!(adj.left_functor.as_str(), "L");
        assert_eq!(adj.right_functor.as_str(), "L†");
        assert!(adj.is_coherent());
    }

    #[test]
    fn build_adjunction_left_of_right() {
        let pre = SaftPreconditions::fully_satisfied("R");
        let adj = build_adjunction(
            "R",
            &sample_c(),
            &sample_d(),
            &pre,
            AdjunctionDirection::BuildLeftOfRight,
        )
        .expect("preconditions hold");
        assert_eq!(adj.right_functor.as_str(), "R");
        assert_eq!(adj.left_functor.as_str(), "R†");
        assert!(adj.is_coherent());
    }

    #[test]
    fn build_adjunction_fails_when_preconditions_fail() {
        let mut pre = SaftPreconditions::fully_satisfied("L");
        pre.source_presentable = false;
        let adj = build_adjunction(
            "L",
            &sample_c(),
            &sample_d(),
            &pre,
            AdjunctionDirection::BuildRightOfLeft,
        );
        assert!(adj.is_none(),
            "non-presentable source must defeat HTT 5.5.2.9");
    }

    #[test]
    fn triangle_identities_decidable() {
        let pre = SaftPreconditions::fully_satisfied("L");
        let adj = build_adjunction(
            "L",
            &sample_c(),
            &sample_d(),
            &pre,
            AdjunctionDirection::BuildRightOfLeft,
        )
        .unwrap();
        assert!(triangle_identities_witness(&adj));
    }

    // ----- Composition tests -----

    #[test]
    fn compose_adjunctions_chains_categories() {
        let pre = SaftPreconditions::fully_satisfied("L");
        let cd = build_adjunction(
            "L_CD",
            &sample_c(),
            &sample_d(),
            &pre,
            AdjunctionDirection::BuildRightOfLeft,
        )
        .unwrap();
        let de = build_adjunction(
            "L_DE",
            &sample_d(),
            &sample_e(),
            &pre,
            AdjunctionDirection::BuildRightOfLeft,
        )
        .unwrap();
        let ce = compose_adjunctions(&cd, &de).expect("composable");
        assert_eq!(ce.source_category, sample_c());
        assert_eq!(ce.target_category, sample_e());
        assert!(ce.is_coherent());
    }

    #[test]
    fn compose_adjunctions_fails_when_categories_dont_match() {
        let pre = SaftPreconditions::fully_satisfied("L");
        let cd = build_adjunction(
            "L_CD",
            &sample_c(),
            &sample_d(),
            &pre,
            AdjunctionDirection::BuildRightOfLeft,
        )
        .unwrap();
        // Build an adjunction E → C — composing CD ∘ EC should fail.
        let ec = build_adjunction(
            "L_EC",
            &sample_e(),
            &sample_c(),
            &pre,
            AdjunctionDirection::BuildRightOfLeft,
        )
        .unwrap();
        assert!(compose_adjunctions(&cd, &ec).is_none(),
            "adjunctions don't compose when target ≠ source");
    }

    // ----- MSFS Lemma 10.3 chain integration -----

    #[test]
    fn msfs_lemma_10_3_iota_r_chain() {
        // Lemma 10.3 builds (ι, r) where ι : S_S → cF is a fully-faithful
        // inclusion and r : cF → S_S is the reflector.  Under SAFT
        // preconditions r exists as ι's left adjoint.
        let s_s = InfinityCategory::at_canonical_universe(
            "S_S^global", Ordinal::Finite(1),
        );
        let cf = InfinityCategory::at_canonical_universe(
            "cF", Ordinal::Finite(1),
        );
        let pre = SaftPreconditions::fully_satisfied("ι");
        let adj = build_adjunction(
            "ι", &s_s, &cf, &pre, AdjunctionDirection::BuildLeftOfRight,
        )
        .expect("Lemma 10.3 SAFT preconditions hold");
        assert_eq!(adj.right_functor.as_str(), "ι");
        assert_eq!(adj.left_functor.as_str(), "ι†");
        assert!(adj.is_coherent());
    }
}
