//! Foundation profile — classification of the logical foundation
//! a corpus of theorems assumes.
//!
//! Verum's "foundation-neutral" claim — that the same proof
//! infrastructure works across **ZFC**, **HoTT** (Homotopy Type
//! Theory), **Cubical** (CCHM cubical type theory), and other
//! logical foundations — requires explicit classification: every
//! theorem must declare which foundation it assumes, and the
//! audit gates / corpus organizers must be able to filter by
//! foundation profile.
//!
//! This module establishes the canonical `FoundationProfile`
//! enum + classification helpers.  It plugs into the existing
//! [`crate::zfc_self_recognition`] machinery for ZFC-specific
//! axiom decomposition while extending the surface to non-ZFC
//! foundations.
//!
//! ## Architectural alignment with Verum philosophy
//!
//! - **Foundation-neutral**: the same kernel rules and certificate
//!   format work across all foundations.  `FoundationProfile`
//!   classifies which foundation a corpus chooses, without
//!   privileging any single one.
//! - **Semantic honesty**: every theorem's foundation is explicit
//!   data — not "the kernel just trusts ZFC".  Cross-foundation
//!   audits surface incompatibilities (e.g., a theorem requiring
//!   univalence ported to a UIP corpus).
//! - **Gradual safety**: foundation-mixed corpora are expressible
//!   (a body of mathematics with both ZFC-tagged and HoTT-tagged
//!   theorems); the audit gate flags theorems that depend on a
//!   foundation incompatible with their consumers.

use serde::{Deserialize, Serialize};

use crate::zfc_self_recognition::InaccessibleLevel;

// =============================================================================
// FoundationProfile
// =============================================================================

/// Logical foundation a corpus assumes.
///
/// **Stable serde tags** (snake_case) for JSON pipelines.
///
/// Variants are ordered by historical adoption: ZFC (set-theoretic,
/// classical) first, then HoTT (type-theoretic with univalence),
/// then Cubical (constructive HoTT with computational univalence),
/// then constructive variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FoundationProfile {
    /// **ZFC + 0 inaccessibles** — pure Zermelo-Fraenkel set theory
    /// with axiom of choice.  Sufficient for first-order logic and
    /// elementary mathematics; insufficient for Grothendieck
    /// universes or (∞,1)-category theory.
    Zfc,
    /// **ZFC + 1 inaccessible (κ_1)** — adds a Grothendieck universe
    /// at level κ_1.  Sufficient for Verum's basic universe tower
    /// and HTT (Higher Topos Theory).
    ZfcOneInaccessible,
    /// **ZFC + 2 inaccessibles (κ_1 < κ_2)** — Verum's default
    /// meta-theory.  Sufficient for the (∞,1)-category meta-classifier
    /// and the universe stratification `Type_0 ∈ Type_1 ∈ Type_2`.
    ZfcTwoInaccessibles,
    /// **ZFC + 3 inaccessibles** — extension for MSFS §11
    /// trinitarian construction.
    ZfcThreeInaccessibles,
    /// **MLTT** — pure Martin-Löf type theory.  No univalence axiom,
    /// no UIP axiom; identity types are intensional.
    Mltt,
    /// **MLTT + UIP** — Martin-Löf type theory extended with
    /// uniqueness of identity proofs.  Compatible with classical
    /// equality reasoning; incompatible with univalence.
    MlttUip,
    /// **HoTT** — Homotopy Type Theory: MLTT + univalence + higher
    /// inductive types.  Identity types are weak (proof-relevant);
    /// univalence is an axiom (no computational rule).
    Hott,
    /// **Cubical** — CCHM Cubical Type Theory.  Constructive
    /// implementation of HoTT: univalence, function extensionality,
    /// and HITs all have computational rules.
    Cubical,
    /// **Predicative MLTT** — MLTT without impredicative universes.
    /// Foundation for predicative mathematics (Bishop-style
    /// constructive analysis).
    PredicativeMltt,
    /// **CIC** — Calculus of Inductive Constructions.  The kernel
    /// of Coq.  Impredicative `Prop` + predicative `Type` hierarchy
    /// + inductive types.
    Cic,
}

impl FoundationProfile {
    /// Stable diagnostic tag — matches the serde representation.
    pub fn tag(self) -> &'static str {
        match self {
            FoundationProfile::Zfc => "zfc",
            FoundationProfile::ZfcOneInaccessible => "zfc_one_inaccessible",
            FoundationProfile::ZfcTwoInaccessibles => "zfc_two_inaccessibles",
            FoundationProfile::ZfcThreeInaccessibles => "zfc_three_inaccessibles",
            FoundationProfile::Mltt => "mltt",
            FoundationProfile::MlttUip => "mltt_uip",
            FoundationProfile::Hott => "hott",
            FoundationProfile::Cubical => "cubical",
            FoundationProfile::PredicativeMltt => "predicative_mltt",
            FoundationProfile::Cic => "cic",
        }
    }

    /// Human-readable display name.
    pub fn display_name(self) -> &'static str {
        match self {
            FoundationProfile::Zfc => "ZFC",
            FoundationProfile::ZfcOneInaccessible => "ZFC + 1 inaccessible (κ₁)",
            FoundationProfile::ZfcTwoInaccessibles => "ZFC + 2 inaccessibles (κ₁ < κ₂)",
            FoundationProfile::ZfcThreeInaccessibles => "ZFC + 3 inaccessibles",
            FoundationProfile::Mltt => "Martin-Löf Type Theory",
            FoundationProfile::MlttUip => "MLTT + UIP",
            FoundationProfile::Hott => "Homotopy Type Theory",
            FoundationProfile::Cubical => "Cubical Type Theory",
            FoundationProfile::PredicativeMltt => "Predicative MLTT",
            FoundationProfile::Cic => "Calculus of Inductive Constructions",
        }
    }

    /// Verum's default foundation — `ZfcTwoInaccessibles`.  Used as
    /// the implicit profile for theorems that don't declare one.
    pub const fn default_profile() -> Self {
        FoundationProfile::ZfcTwoInaccessibles
    }

    /// Whether this profile is set-theoretic (ZFC family).
    pub fn is_set_theoretic(self) -> bool {
        matches!(
            self,
            FoundationProfile::Zfc
                | FoundationProfile::ZfcOneInaccessible
                | FoundationProfile::ZfcTwoInaccessibles
                | FoundationProfile::ZfcThreeInaccessibles
        )
    }

    /// Whether this profile is type-theoretic (MLTT / HoTT / Cubical
    /// / CIC family).
    pub fn is_type_theoretic(self) -> bool {
        matches!(
            self,
            FoundationProfile::Mltt
                | FoundationProfile::MlttUip
                | FoundationProfile::Hott
                | FoundationProfile::Cubical
                | FoundationProfile::PredicativeMltt
                | FoundationProfile::Cic
        )
    }

    /// Whether this profile is constructive (no LEM by default,
    /// computational univalence where applicable).
    pub fn is_constructive(self) -> bool {
        matches!(
            self,
            FoundationProfile::Mltt
                | FoundationProfile::Cubical
                | FoundationProfile::PredicativeMltt
        )
    }

    /// Whether this profile assumes UIP (uniqueness of identity proofs).
    /// UIP is incompatible with univalence.
    pub fn assumes_uip(self) -> bool {
        matches!(self, FoundationProfile::MlttUip | FoundationProfile::Cic)
    }

    /// Whether this profile assumes univalence (the type-theoretic
    /// equivalence-equals-equality axiom).  Univalence is
    /// incompatible with UIP.
    pub fn assumes_univalence(self) -> bool {
        matches!(self, FoundationProfile::Hott | FoundationProfile::Cubical)
    }

    /// Whether this profile is **incompatible** with another — they
    /// can't both be assumed simultaneously.  Used by the
    /// `--framework-conflicts` audit gate.
    ///
    /// Conflict cases:
    ///   - UIP + univalence are mutually exclusive.
    pub fn conflicts_with(self, other: FoundationProfile) -> bool {
        if self.assumes_uip() && other.assumes_univalence() {
            return true;
        }
        if self.assumes_univalence() && other.assumes_uip() {
            return true;
        }
        false
    }

    /// Number of Grothendieck universes (inaccessible cardinals)
    /// this profile requires.  Returns 0 for non-ZFC profiles
    /// (which use type-theoretic universe hierarchies instead).
    pub fn required_inaccessibles(self) -> usize {
        match self {
            FoundationProfile::Zfc => 0,
            FoundationProfile::ZfcOneInaccessible => 1,
            FoundationProfile::ZfcTwoInaccessibles => 2,
            FoundationProfile::ZfcThreeInaccessibles => 3,
            _ => 0,
        }
    }

    /// Iterate the explicit ZFC inaccessibles this profile requires.
    /// Empty for non-ZFC profiles.  Reuses
    /// [`crate::zfc_self_recognition::InaccessibleLevel`] for
    /// integration with the existing self-recognition audit.
    pub fn required_zfc_inaccessibles(self) -> Vec<InaccessibleLevel> {
        match self {
            FoundationProfile::ZfcOneInaccessible => vec![InaccessibleLevel::Kappa1],
            FoundationProfile::ZfcTwoInaccessibles => {
                vec![InaccessibleLevel::Kappa1, InaccessibleLevel::Kappa2]
            }
            FoundationProfile::ZfcThreeInaccessibles => {
                vec![
                    InaccessibleLevel::Kappa1,
                    InaccessibleLevel::Kappa2,
                    // Kappa3 lands in V1 — currently absent from the
                    // InaccessibleLevel enum; future-proof the call.
                ]
            }
            _ => Vec::new(),
        }
    }

    /// All known foundation profiles, in canonical order.  Used by
    /// the audit gate's "list-all-foundations" emission.
    pub fn all() -> [FoundationProfile; 10] {
        [
            FoundationProfile::Zfc,
            FoundationProfile::ZfcOneInaccessible,
            FoundationProfile::ZfcTwoInaccessibles,
            FoundationProfile::ZfcThreeInaccessibles,
            FoundationProfile::Mltt,
            FoundationProfile::MlttUip,
            FoundationProfile::Hott,
            FoundationProfile::Cubical,
            FoundationProfile::PredicativeMltt,
            FoundationProfile::Cic,
        ]
    }
}

impl Default for FoundationProfile {
    fn default() -> Self {
        Self::default_profile()
    }
}

impl std::fmt::Display for FoundationProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_profiles_have_distinct_tags() {
        let tags: std::collections::BTreeSet<_> =
            FoundationProfile::all().iter().map(|p| p.tag()).collect();
        assert_eq!(tags.len(), FoundationProfile::all().len());
    }

    #[test]
    fn all_profiles_have_distinct_display_names() {
        let names: std::collections::BTreeSet<_> = FoundationProfile::all()
            .iter()
            .map(|p| p.display_name())
            .collect();
        assert_eq!(names.len(), FoundationProfile::all().len());
    }

    #[test]
    fn default_profile_is_zfc_two_inaccessibles() {
        assert_eq!(
            FoundationProfile::default(),
            FoundationProfile::ZfcTwoInaccessibles,
        );
    }

    #[test]
    fn set_theoretic_classification() {
        assert!(FoundationProfile::Zfc.is_set_theoretic());
        assert!(FoundationProfile::ZfcTwoInaccessibles.is_set_theoretic());
        assert!(!FoundationProfile::Hott.is_set_theoretic());
        assert!(!FoundationProfile::Cubical.is_set_theoretic());
    }

    #[test]
    fn type_theoretic_classification() {
        assert!(FoundationProfile::Mltt.is_type_theoretic());
        assert!(FoundationProfile::Hott.is_type_theoretic());
        assert!(FoundationProfile::Cubical.is_type_theoretic());
        assert!(FoundationProfile::Cic.is_type_theoretic());
        assert!(!FoundationProfile::Zfc.is_type_theoretic());
    }

    #[test]
    fn constructive_classification() {
        assert!(FoundationProfile::Mltt.is_constructive());
        assert!(FoundationProfile::Cubical.is_constructive());
        assert!(FoundationProfile::PredicativeMltt.is_constructive());
        assert!(!FoundationProfile::Hott.is_constructive(), "HoTT axiomatic univalence breaks constructivity");
        assert!(!FoundationProfile::Zfc.is_constructive());
    }

    #[test]
    fn uip_only_for_mltt_uip_and_cic() {
        assert!(FoundationProfile::MlttUip.assumes_uip());
        assert!(FoundationProfile::Cic.assumes_uip());
        assert!(!FoundationProfile::Mltt.assumes_uip());
        assert!(!FoundationProfile::Hott.assumes_uip());
    }

    #[test]
    fn univalence_only_for_hott_and_cubical() {
        assert!(FoundationProfile::Hott.assumes_univalence());
        assert!(FoundationProfile::Cubical.assumes_univalence());
        assert!(!FoundationProfile::Mltt.assumes_univalence());
        assert!(!FoundationProfile::MlttUip.assumes_univalence());
        assert!(!FoundationProfile::Zfc.assumes_univalence());
    }

    #[test]
    fn uip_and_univalence_conflict() {
        assert!(FoundationProfile::MlttUip.conflicts_with(FoundationProfile::Hott));
        assert!(FoundationProfile::Hott.conflicts_with(FoundationProfile::MlttUip));
        assert!(FoundationProfile::Cic.conflicts_with(FoundationProfile::Cubical));
        assert!(FoundationProfile::Cubical.conflicts_with(FoundationProfile::Cic));
    }

    #[test]
    fn compatible_profiles_dont_conflict() {
        assert!(!FoundationProfile::Mltt.conflicts_with(FoundationProfile::Hott));
        assert!(!FoundationProfile::Mltt.conflicts_with(FoundationProfile::Cubical));
        assert!(!FoundationProfile::Zfc.conflicts_with(FoundationProfile::ZfcOneInaccessible));
        assert!(!FoundationProfile::Hott.conflicts_with(FoundationProfile::Cubical));
    }

    #[test]
    fn required_inaccessibles_count_matches_variant() {
        assert_eq!(FoundationProfile::Zfc.required_inaccessibles(), 0);
        assert_eq!(FoundationProfile::ZfcOneInaccessible.required_inaccessibles(), 1);
        assert_eq!(FoundationProfile::ZfcTwoInaccessibles.required_inaccessibles(), 2);
        assert_eq!(FoundationProfile::ZfcThreeInaccessibles.required_inaccessibles(), 3);
        assert_eq!(FoundationProfile::Hott.required_inaccessibles(), 0);
    }

    #[test]
    fn required_zfc_inaccessibles_returns_correct_levels() {
        assert!(FoundationProfile::Zfc.required_zfc_inaccessibles().is_empty());
        assert_eq!(
            FoundationProfile::ZfcOneInaccessible.required_zfc_inaccessibles(),
            vec![InaccessibleLevel::Kappa1],
        );
        assert_eq!(
            FoundationProfile::ZfcTwoInaccessibles.required_zfc_inaccessibles(),
            vec![InaccessibleLevel::Kappa1, InaccessibleLevel::Kappa2],
        );
    }

    #[test]
    fn serde_round_trip_for_every_variant() {
        for profile in FoundationProfile::all() {
            let json = serde_json::to_string(&profile).unwrap();
            let restored: FoundationProfile = serde_json::from_str(&json).unwrap();
            assert_eq!(restored, profile, "serde round-trip failed for {:?}", profile);
        }
    }

    #[test]
    fn display_uses_human_readable_name() {
        let s = format!("{}", FoundationProfile::Hott);
        assert_eq!(s, "Homotopy Type Theory");
        let s = format!("{}", FoundationProfile::ZfcTwoInaccessibles);
        assert!(s.contains("κ"));
    }

    #[test]
    fn type_and_set_theoretic_are_mutually_exclusive() {
        for profile in FoundationProfile::all() {
            let s = profile.is_set_theoretic();
            let t = profile.is_type_theoretic();
            assert!(
                !(s && t),
                "{:?} cannot be both set-theoretic and type-theoretic",
                profile,
            );
        }
    }
}
