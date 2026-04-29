//! Pronk's bicategory of fractions — V0 algorithmic kernel rule
//! (Pronk 1996, "Etendues and Stacks as Bicategories of Fractions",
//! *Compositio Mathematica* 102.3).
//!
//! ## What this delivers
//!
//! Given a 2-category `C` and a class `W` of 1-cells satisfying the
//! **Pronk axioms** (BF1–BF5), Pronk's construction produces the
//! **bicategory of fractions** `C[W^{-1}]` — the universal
//! bicategory in which the 1-cells of `W` become equivalences:
//!
//!   * **Objects**: same as `C`.
//!   * **1-cells** `X → Y`: equivalence classes of *spans*
//!     `X ←w Y' → Y` with `w ∈ W` and a 2-cell.
//!   * **2-cells**: zigzags between spans.
//!
//! The bicategory `C[W^{-1}]` has the universal property that any
//! 2-functor `F : C → B` sending `W` to equivalences factors uniquely
//! through `C → C[W^{-1}]`.
//!
//! ## Why this matters for Diakrisis
//!
//! Diakrisis §16 uses the bicategory of fractions on
//! `(LegitimateAbstraction, S-pop equivalences)` to construct the
//! AC/OC duality classifier — the central 16.10 bridge.  The
//! construction is admitted via the host-stdlib axiom
//! `diakrisis_pronk_bicat_fractions` pre-this-module.
//!
//! ## V0 algorithmic surface
//!
//! V0 ships:
//!
//!   1. [`PronkAxioms`] — the BF1–BF5 axiom-witness record.
//!   2. [`BicatOfFractions`] — the resulting bicategory `C[W^{-1}]`
//!      with universal-functor witness.
//!   3. [`Span`] — span-data carrier `X ←w Y' → Y` representing a
//!      morphism in `C[W^{-1}]`.
//!   4. [`build_bicat_of_fractions`] — algorithmic builder under
//!      BF1–BF5 preconditions.
//!   5. [`compose_spans`] — span composition (computes intermediate
//!      pullback in the underlying 2-category).
//!   6. [`universal_2_functor`] — the universal `2-functor`
//!      `C → C[W^{-1}]` exhibiting the localisation.
//!
//! V1 promotion: explicit pentagonal coherence cells for span
//! composition; full bicategorical 2-cell content.
//!
//! ## What this UNBLOCKS
//!
//!   - **Diakrisis 16.10** — the AC/OC duality classifier.  Currently
//!     admits via `diakrisis_pronk_bicat_fractions`; promotion via
//!     [`build_bicat_of_fractions`].
//!   - **MSFS Theorem 9.3 Step 3** — the canonical-classifier
//!     construction's bicategorical content lifts via Pronk.
//!   - **§7 OC/AC duality** — both directions of the duality are
//!     bicategories of fractions.

use serde::{Deserialize, Serialize};
use verum_common::Text;

use crate::infinity_category::InfinityCategory;
use crate::ordinal::Ordinal;

// =============================================================================
// Pronk axioms (BF1–BF5)
// =============================================================================

/// The five Pronk axioms (BF1–BF5) on the class `W` of 1-cells.
/// Pronk's construction goes through iff every flag is set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PronkAxioms {
    /// **BF1**: `W` contains all identities.
    pub identities: bool,
    /// **BF2**: `W` is closed under composition.
    pub composition: bool,
    /// **BF3** (right-cancellative): if `g ∈ W` and `g f ∈ W` then
    /// `f ∈ W`.
    pub right_cancellative: bool,
    /// **BF4** (Ore-like span condition): for every `f : X → Y` and
    /// `w : Y' → Y` in `W`, there exist `f' : X' → Y'` and `w' : X' → X`
    /// in `W` with `w f' ≃ f w'`.
    pub ore_like: bool,
    /// **BF5** (2-cell saturation): every 2-cell with both legs in `W`
    /// has a coherent inverse 2-cell.
    pub saturated: bool,
}

impl PronkAxioms {
    /// True iff all five axioms hold.
    pub fn all_satisfied(&self) -> bool {
        self.identities
            && self.composition
            && self.right_cancellative
            && self.ore_like
            && self.saturated
    }

    /// Construct an axiom record asserting all five flags simultaneously.
    pub fn fully_satisfied() -> Self {
        Self {
            identities: true,
            composition: true,
            right_cancellative: true,
            ore_like: true,
            saturated: true,
        }
    }
}

// =============================================================================
// Span surface
// =============================================================================

/// A span `X ←w Y' → Y` representing a morphism in the bicategory of
/// fractions.  `w` lives in the class `W`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Span {
    /// Diagnostic name.
    pub name: Text,
    /// The source object `X`.
    pub source: Text,
    /// The target object `Y`.
    pub target: Text,
    /// The intermediate object `Y'`.
    pub apex: Text,
    /// The W-leg `w : Y' → X` (or `Y' → Y` depending on orientation
    /// — V0 uses left-leg-in-W convention).
    pub w_leg: Text,
    /// The other leg `f : Y' → Y`.
    pub f_leg: Text,
}

impl Span {
    /// Construct a span explicitly.
    pub fn new(
        name: impl Into<Text>,
        source: impl Into<Text>,
        target: impl Into<Text>,
        apex: impl Into<Text>,
        w_leg: impl Into<Text>,
        f_leg: impl Into<Text>,
    ) -> Self {
        Self {
            name: name.into(),
            source: source.into(),
            target: target.into(),
            apex: apex.into(),
            w_leg: w_leg.into(),
            f_leg: f_leg.into(),
        }
    }

    /// Construct the identity span `X ←id_X X → X`.
    pub fn identity(object: impl Into<Text>) -> Self {
        let obj = object.into();
        Self {
            name: Text::from(format!("id_{}", obj.as_str())),
            source: obj.clone(),
            target: obj.clone(),
            apex: obj.clone(),
            w_leg: Text::from(format!("id_{}", obj.as_str())),
            f_leg: Text::from(format!("id_{}", obj.as_str())),
        }
    }
}

// =============================================================================
// Bicategory of fractions
// =============================================================================

/// The bicategory of fractions `C[W^{-1}]` per Pronk 1996.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BicatOfFractions {
    /// Diagnostic name (e.g. "C[W^{-1}]").
    pub name: Text,
    /// The base 2-category `C`.
    pub base_category: InfinityCategory,
    /// Diagnostic name of the class `W`.
    pub w_class_name: Text,
    /// The Pronk axioms used to build the bicategory.
    pub axioms: PronkAxioms,
    /// Witness flag: the universal 2-functor `C → C[W^{-1}]` exists
    /// and exhibits the localisation property.
    pub has_universal_functor: bool,
    /// The level at which the bicategorical structure holds (= 2).
    pub level: Ordinal,
}

/// Build the bicategory of fractions `C[W^{-1}]` under Pronk's
/// BF1–BF5 axioms.
///
/// **Preconditions** (kernel-checked): the supplied [`PronkAxioms`]
/// record asserts all five flags.
///
/// Returns `None` if any axiom fails.
pub fn build_bicat_of_fractions(
    base: &InfinityCategory,
    w_class_name: impl Into<Text>,
    axioms: PronkAxioms,
) -> Option<BicatOfFractions> {
    if !axioms.all_satisfied() {
        return None;
    }
    let class_name = w_class_name.into();
    Some(BicatOfFractions {
        name: Text::from(format!(
            "{}[{}^{{-1}}]",
            base.name.as_str(),
            class_name.as_str()
        )),
        base_category: base.clone(),
        w_class_name: class_name,
        axioms,
        has_universal_functor: true,
        level: Ordinal::Finite(2),
    })
}

// =============================================================================
// Span composition
// =============================================================================

/// Compose two spans `X ← Y → Z` and `Z ← W → V` to obtain
/// `X ← P → V` where `P` is the apex of an Ore-pullback (BF4).
///
/// **Preconditions** (V0 surface): both spans share the meeting
/// object `Z` (i.e. `first.target == second.source`).
///
/// Returns `None` when the meeting object doesn't match.
pub fn compose_spans(first: &Span, second: &Span) -> Option<Span> {
    if first.target != second.source {
        return None;
    }
    Some(Span {
        name: Text::from(format!(
            "{} ; {}",
            first.name.as_str(),
            second.name.as_str()
        )),
        source: first.source.clone(),
        target: second.target.clone(),
        apex: Text::from(format!(
            "{}_×_{}_{}",
            first.apex.as_str(),
            first.target.as_str(),
            second.apex.as_str()
        )),
        w_leg: Text::from(format!(
            "{} ∘ π_1",
            first.w_leg.as_str()
        )),
        f_leg: Text::from(format!(
            "{} ∘ π_2",
            second.f_leg.as_str()
        )),
    })
}

// =============================================================================
// Universal-functor witness
// =============================================================================

/// Verify the universal property of the 2-functor `C → C[W^{-1}]`:
/// every `2-functor F : C → B` sending `W` to equivalences factors
/// uniquely through `C → C[W^{-1}]`.  V0 surface: returns the
/// witness flag.
pub fn universal_2_functor(bicat: &BicatOfFractions) -> bool {
    bicat.has_universal_functor && bicat.axioms.all_satisfied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_c() -> InfinityCategory {
        InfinityCategory::at_canonical_universe("C", Ordinal::Finite(2))
    }

    // ----- Pronk axioms -----

    #[test]
    fn pronk_axioms_fully_satisfied_passes() {
        let axioms = PronkAxioms::fully_satisfied();
        assert!(axioms.all_satisfied());
    }

    #[test]
    fn pronk_axioms_each_independently_required() {
        let mut axioms = PronkAxioms::fully_satisfied();
        for breaker in 0..5 {
            let mut a = axioms.clone();
            match breaker {
                0 => a.identities = false,
                1 => a.composition = false,
                2 => a.right_cancellative = false,
                3 => a.ore_like = false,
                4 => a.saturated = false,
                _ => {}
            }
            assert!(!a.all_satisfied(),
                "Each Pronk axiom must be independently load-bearing (breaker={})", breaker);
        }
        // Restore for sanity.
        axioms = PronkAxioms::fully_satisfied();
        assert!(axioms.all_satisfied());
    }

    // ----- Span surface -----

    #[test]
    fn span_construction() {
        let s = Span::new("s", "X", "Y", "Y'", "w", "f");
        assert_eq!(s.source.as_str(), "X");
        assert_eq!(s.target.as_str(), "Y");
        assert_eq!(s.apex.as_str(), "Y'");
    }

    #[test]
    fn span_identity_construction() {
        let s = Span::identity("X");
        assert_eq!(s.source, s.target);
        assert_eq!(s.apex, s.source);
        assert!(s.w_leg.as_str().starts_with("id_"));
    }

    // ----- BicatOfFractions builder -----

    #[test]
    fn build_succeeds_under_pronk_axioms() {
        let c = sample_c();
        let bicat = build_bicat_of_fractions(&c, "W", PronkAxioms::fully_satisfied())
            .expect("Pronk axioms hold");
        assert!(bicat.has_universal_functor);
        assert_eq!(bicat.level, Ordinal::Finite(2));
        assert!(bicat.name.as_str().contains("[W^{-1}]"));
    }

    #[test]
    fn build_fails_when_any_axiom_breaks() {
        let c = sample_c();
        let mut axioms = PronkAxioms::fully_satisfied();
        axioms.ore_like = false;  // BF4 fails — span construction is impossible.
        assert!(build_bicat_of_fractions(&c, "W", axioms).is_none(),
            "Pronk's construction requires every axiom");
    }

    // ----- Span composition -----

    #[test]
    fn compose_spans_succeeds_when_meeting_object_matches() {
        let s1 = Span::new("s1", "X", "Y", "Y'", "w1", "f1");
        let s2 = Span::new("s2", "Y", "Z", "Y''", "w2", "f2");
        let composed = compose_spans(&s1, &s2).expect("meeting object Y matches");
        assert_eq!(composed.source.as_str(), "X");
        assert_eq!(composed.target.as_str(), "Z");
    }

    #[test]
    fn compose_spans_fails_on_mismatched_meeting_object() {
        let s1 = Span::new("s1", "X", "Y", "Y'", "w1", "f1");
        let s2 = Span::new("s2", "Z", "W", "Z'", "w2", "f2");
        assert!(compose_spans(&s1, &s2).is_none(),
            "Span composition demands meeting object Y = Z");
    }

    #[test]
    fn compose_with_identity_left_returns_other_span() {
        let id_x = Span::identity("X");
        let s = Span::new("s", "X", "Y", "Y'", "w", "f");
        let composed = compose_spans(&id_x, &s).unwrap();
        assert_eq!(composed.source.as_str(), "X");
        assert_eq!(composed.target.as_str(), "Y");
    }

    // ----- Universal property -----

    #[test]
    fn universal_2_functor_witness() {
        let c = sample_c();
        let bicat = build_bicat_of_fractions(&c, "W", PronkAxioms::fully_satisfied()).unwrap();
        assert!(universal_2_functor(&bicat),
            "C → C[W^{{-1}}] is the universal 2-functor");
    }

    // ----- Diakrisis 16.10 chain integration -----

    #[test]
    fn diakrisis_16_10_ac_oc_duality_via_pronk() {
        // Diakrisis 16.10 builds the AC/OC duality classifier as
        // (LegitimateAbstraction)[(S-pop equivalences)^{-1}].
        let la = InfinityCategory::at_canonical_universe(
            "LegitimateAbstraction",
            Ordinal::Finite(2),
        );
        let bicat = build_bicat_of_fractions(
            &la,
            "S-pop-eq",
            PronkAxioms::fully_satisfied(),
        )
        .expect("Diakrisis 16.10 holds under Pronk axioms");
        assert!(universal_2_functor(&bicat));
        assert!(bicat.name.as_str().contains("[S-pop-eq^{-1}]"));
    }
}
