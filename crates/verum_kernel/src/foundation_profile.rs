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
//! enum + classification helpers. It plugs into the existing
//! [`crate::zfc_self_recognition`] machinery for ZFC-specific
//! axiom decomposition while extending the surface to non-ZFC
//! foundations.
//!

//! ## Architectural alignment with Verum philosophy
//!

//! - **Foundation-neutral**: the same kernel rules and certificate
//!  format work across all foundations. `FoundationProfile`
//!  classifies which foundation a corpus chooses, without
//!  privileging any single one.
//! - **Semantic honesty**: every theorem's foundation is explicit
//!  data — not "the kernel just trusts ZFC". Cross-foundation
//!  audits surface incompatibilities (e.g., a theorem requiring
//!  univalence ported to a UIP corpus).
//! - **Gradual safety**: foundation-mixed corpora are expressible
//!  (a body of mathematics with both ZFC-tagged and HoTT-tagged
//!  theorems); the audit gate flags theorems that depend on a
//!  foundation incompatible with their consumers.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::framework_citation::{FrameworkCitation, FrameworkCitationManifest};
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FoundationProfile {
    /// **ZFC + 0 inaccessibles** — pure Zermelo-Fraenkel set theory
    /// with axiom of choice. Sufficient for first-order logic and
    /// elementary mathematics; insufficient for Grothendieck
    /// universes or (∞,1)-category theory.
    Zfc,
    /// **ZFC + 1 inaccessible (κ_1)** — adds a Grothendieck universe
    /// at level κ_1. Sufficient for Verum's basic universe tower
    /// and HTT (Higher Topos Theory).
    ZfcOneInaccessible,
    /// **ZFC + 2 inaccessibles (κ_1 < κ_2)** — Verum's default
    /// meta-theory. Sufficient for the (∞,1)-category meta-classifier
    /// and the universe stratification `Type_0 ∈ Type_1 ∈ Type_2`.
    ZfcTwoInaccessibles,
    /// **ZFC + 3 inaccessibles** — extension for MSFS §11
    /// trinitarian construction.
    ZfcThreeInaccessibles,
    /// **MLTT** — pure Martin-Löf type theory. No univalence axiom,
    /// no UIP axiom; identity types are intensional.
    Mltt,
    /// **MLTT + UIP** — Martin-Löf type theory extended with
    /// uniqueness of identity proofs. Compatible with classical
    /// equality reasoning; incompatible with univalence.
    MlttUip,
    /// **HoTT** — Homotopy Type Theory: MLTT + univalence + higher
    /// inductive types. Identity types are weak (proof-relevant);
    /// univalence is an axiom (no computational rule).
    Hott,
    /// **Cubical** — CCHM Cubical Type Theory. Constructive
    /// implementation of HoTT: univalence, function extensionality,
    /// and HITs all have computational rules.
    Cubical,
    /// **Predicative MLTT** — MLTT without impredicative universes.
    /// Foundation for predicative mathematics (Bishop-style
    /// constructive analysis).
    PredicativeMltt,
    /// **CIC** — Calculus of Inductive Constructions. The kernel
    /// of Coq. Impredicative `Prop` + predicative `Type` hierarchy
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

    /// Verum's default foundation — `ZfcTwoInaccessibles`. Used as
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
    /// equivalence-equals-equality axiom). Univalence is
    /// incompatible with UIP.
    pub fn assumes_univalence(self) -> bool {
        matches!(self, FoundationProfile::Hott | FoundationProfile::Cubical)
    }

    /// Whether this profile is **incompatible** with another — they
    /// can't both be assumed simultaneously. Used by the
    /// `--framework-conflicts` audit gate.
    ///

    /// Conflict cases:
    ///  - UIP + univalence are mutually exclusive.
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
    /// this profile requires. Returns 0 for non-ZFC profiles
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
    /// Empty for non-ZFC profiles. Reuses
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

    /// **Bridge to existing `@framework(<tag>, "...")` citations** in
    /// `core/math/`. Maps a citation tag (as written in the
    /// `@framework` attribute argument) to its foundation profile.
    ///

    /// Recognised tags from the existing corpus:
    ///

    ///  - `"hott"` → `FoundationProfile::Hott`
    ///  - `"cubical"` → `FoundationProfile::Cubical`
    ///  - `"zfc"` → `FoundationProfile::Zfc` (default — no inaccessibles)
    ///  - `"zfc_one_inaccessible"` / `"htt"` (Lurie HTT requires
    ///  ZFC + 1 inaccessible) → `FoundationProfile::ZfcOneInaccessible`
    ///  - `"zfc_two_inaccessibles"` / `"msfs"` (MSFS requires ZFC + 2
    ///  inaccessibles) → `FoundationProfile::ZfcTwoInaccessibles`
    ///  - `"mltt"` → `FoundationProfile::Mltt`
    ///  - `"mltt_uip"` / `"uip"` → `FoundationProfile::MlttUip`
    ///  - `"cic"` / `"coq"` → `FoundationProfile::Cic`
    ///  - `"predicative_mltt"` / `"predicative"` → `FoundationProfile::PredicativeMltt`
    ///

    /// Tags not in this list (e.g., framework names like
    /// `"lurie_htt"`, `"schreiber_dcct"`, `"baez_dolan"`) are
    /// FRAMEWORKS WITHIN a foundation — they cite specific results
    /// in the foundation's literature, not the foundation itself.
    /// `from_framework_tag` returns `None` for those; the consumer
    /// classifies via the framework's known foundation separately.
    pub fn from_framework_tag(tag: &str) -> Option<Self> {
        match tag {
            "hott" => Some(FoundationProfile::Hott),
            "cubical" => Some(FoundationProfile::Cubical),
            "zfc" => Some(FoundationProfile::Zfc),
            "zfc_one_inaccessible" | "htt" => Some(FoundationProfile::ZfcOneInaccessible),
            "zfc_two_inaccessibles" | "msfs" => Some(FoundationProfile::ZfcTwoInaccessibles),
            "zfc_three_inaccessibles" => Some(FoundationProfile::ZfcThreeInaccessibles),
            "mltt" => Some(FoundationProfile::Mltt),
            "mltt_uip" | "uip" => Some(FoundationProfile::MlttUip),
            "cic" | "coq" => Some(FoundationProfile::Cic),
            "predicative_mltt" | "predicative" => Some(FoundationProfile::PredicativeMltt),
            _ => None,
        }
    }

    /// **Framework → foundation map** for citations naming a
    /// specific body of mathematical literature. Where
    /// [`from_framework_tag`](Self::from_framework_tag) recognises
    /// foundation-level tags (`"hott"`, `"cubical"`, `"zfc"`),
    /// this method recognises FRAMEWORK-level tags (specific
    /// corpora WITHIN a foundation: `"lurie_htt"`,
    /// `"schreiber_dcct"`, `"baez_dolan"`, …) and returns the
    /// foundation each framework lives in.
    ///

    /// **Recognised frameworks** (drawn from the actual `core/math/`
    /// corpus inventory — `verum audit --framework-axioms` lists every
    /// citation):
    ///

    /// ZFC + 2 inaccessibles family:
    ///  - `"msfs"` (107 uses) — Moduli Space of Formal Systems.
    ///  - `"diakrisis"` (53 uses) — Yanofsky-style self-reference
    ///  paradox-blocking.
    ///  - `"connes_reconstruction"` (8) — non-commutative geometry.
    ///  - `"baez_dolan"` (4) — n-category cobordism hypothesis.
    ///  - `"schreiber_dcct"` (5) — differential cohesive ∞-topos.
    ///  - `"petz_classification"` (4) — quantum-information ordering.
    ///  - `"adamek_rosicky"` (3) — locally-presentable categories.
    ///  - `"lair_makkai_pare"` — accessibility theory.
    ///  - `"lambek_scott"` — cartesian-closed categories ↔ STLC.
    ///

    /// ZFC + 1 inaccessible:
    ///  - `"lurie_htt"` (11 uses) — Higher Topos Theory.
    ///

    /// ZFC (no inaccessibles needed):
    ///  - `"arnold_catastrophe"` (8) — singularity theory.
    ///  - `"bounded_arithmetic_*"` (~10 uses) — proof-complexity
    ///  fragments (I_Δ_0 / S_2^1 / V_0 / V_1 / V_NP / V_PH).
    ///

    /// Domain-specific (return `None` — not foundations):
    ///  - `"owl2_fs"` (66 uses) — OWL 2 functional syntax (DL fragment).
    ///

    /// Unknown tags return `None`.
    pub fn from_known_framework(framework: &str) -> Option<Self> {
        match framework {
            // ZFC + 2 inaccessibles (Verum's default meta-theory).
            // `verum` is the self-citation tag — corpus theorems that
            // cite a Verum-side stdlib result use `@framework(verum,
            // "...")` to mark the trust dependency on Verum's own
            // proven body.
            "msfs"
            | "diakrisis"
            | "connes_reconstruction"
            | "baez_dolan"
            | "schreiber_dcct"
            | "petz_classification"
            | "adamek_rosicky"
            | "lair_makkai_pare"
            | "lambek_scott"
            | "verum" => Some(FoundationProfile::ZfcTwoInaccessibles),
            // ZFC + 1 inaccessible (HTT lives here).
            "lurie_htt" => Some(FoundationProfile::ZfcOneInaccessible),
            // CIC (Coq + Lean 4 + their stdlibs all share the
            // CIC/CoIC family — Lean 4's logic is a CIC-derived
            // dependent type theory; mathlib4 lives in Lean 4).
            // Foundation-classification operates at the family
            // granularity, so they all map to Cic.
            "coq_stdlib" | "lean4_stdlib" | "mathlib4" => {
                Some(FoundationProfile::Cic)
            }
            // Pure ZFC (elementary mathematics, no Grothendieck universes).
            "arnold_catastrophe"
            | "bounded_arithmetic_i_delta_0"
            | "bounded_arithmetic_s_2_1"
            | "bounded_arithmetic_v_0"
            | "bounded_arithmetic_v_1"
            | "bounded_arithmetic_v_np"
            | "bounded_arithmetic_v_ph" => Some(FoundationProfile::Zfc),
            _ => None,
        }
    }

    /// **Comprehensive resolver**: try the foundation-tag bridge first
    /// ([`from_framework_tag`](Self::from_framework_tag)), fall back
    /// to the framework-name bridge
    /// ([`from_known_framework`](Self::from_known_framework)).
    /// Returns `None` only when neither recognises the tag.
    ///

    /// This is the canonical entry point for "given a citation
    /// `@framework(<tag>, ...)`, what foundation does it imply?".
    pub fn resolve_citation(tag: &str) -> Option<Self> {
        Self::from_framework_tag(tag).or_else(|| Self::from_known_framework(tag))
    }

    /// All known foundation profiles, in canonical order. Used by
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

// =============================================================================
// FoundationDistribution — bridge to FrameworkCitationManifest
// =============================================================================

/// One citation that couldn't be classified into a foundation by
/// either [`FoundationProfile::from_framework_tag`] or
/// [`FoundationProfile::from_known_framework`].
///

/// Surfaced by the audit gate so the corpus author can either add
/// the framework to the recognised set or correct the citation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedCitation {
    /// Framework name as written in the `@framework(...)` attribute.
    pub framework: String,
    /// Citation string (path / paragraph reference).
    pub citation: String,
    /// Declaration carrying the citation.
    pub decl_name: String,
    /// `"theorem"` / `"lemma"` / `"corollary"` / `"axiom"`.
    pub decl_kind: String,
}

/// One detected cross-foundation incompatibility — typically the
/// UIP-vs-univalence pair, but the data shape is general so future
/// rules (e.g., classical-vs-anti-classical) plug in without API
/// change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundationConflict {
    /// First foundation in the conflicting pair.
    pub left: FoundationProfile,
    /// Second foundation in the conflicting pair.
    pub right: FoundationProfile,
    /// Human-readable explanation.
    pub reason: String,
}

/// Distribution of `@framework(...)` citations across foundation
/// profiles, with conflict detection.
///

/// The data layer underneath
/// `verum audit --foundation-profiles`: takes the citation manifest
/// produced by [`crate::framework_citation::collect_framework_citations`]
/// and partitions it by foundation, surfacing unresolved citations
/// and cross-foundation conflicts.
///

/// **Why a separate analyzer (vs. a method on `FrameworkCitationManifest`)**:
/// the citation manifest knows nothing about foundations — it's
/// purely structural. The classification is a separate concern that
/// lives where foundations are defined. This keeps
/// `framework_citation` reusable for audit gates that don't care
/// about foundations (e.g., `--framework-axioms` enumeration).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FoundationDistribution {
    /// Per-foundation citation count. Keys are foundation profiles
    /// that appeared at least once; missing keys mean zero citations.
    pub by_foundation: BTreeMap<FoundationProfile, usize>,
    /// Citations whose framework name matched neither bridge.
    pub unresolved: Vec<UnresolvedCitation>,
    /// Cross-foundation incompatibilities detected (UIP + univalence
    /// is the canonical case).
    pub conflicts: Vec<FoundationConflict>,
}

impl FoundationDistribution {
    /// Classify every citation in the manifest into a foundation,
    /// accumulating the per-foundation count and recording
    /// unresolved citations. Detects pairwise conflicts among the
    /// distinct foundations present.
    pub fn from_manifest(manifest: &FrameworkCitationManifest) -> Self {
        Self::from_citations(&manifest.rows)
    }

    /// Like [`Self::from_manifest`] but takes the citation list
    /// directly — useful when the caller already iterated rows.
    pub fn from_citations(rows: &[FrameworkCitation]) -> Self {
        let mut by_foundation: BTreeMap<FoundationProfile, usize> = BTreeMap::new();
        let mut unresolved: Vec<UnresolvedCitation> = Vec::new();

        for row in rows {
            match FoundationProfile::resolve_citation(&row.framework) {
                Some(profile) => {
                    *by_foundation.entry(profile).or_insert(0) += 1;
                }
                None => unresolved.push(UnresolvedCitation {
                    framework: row.framework.clone(),
                    citation: row.citation.clone(),
                    decl_name: row.decl_name.clone(),
                    decl_kind: row.decl_kind.clone(),
                }),
            }
        }

        let conflicts = detect_pairwise_conflicts(&by_foundation);

        Self {
            by_foundation,
            unresolved,
            conflicts,
        }
    }

    /// Distinct foundations present (excluding unresolved citations),
    /// in canonical (BTreeMap) order.
    pub fn foundations(&self) -> Vec<FoundationProfile> {
        self.by_foundation.keys().copied().collect()
    }

    /// Total citations that classified into a foundation.
    pub fn resolved_count(&self) -> usize {
        self.by_foundation.values().sum()
    }

    /// Total unresolved citations.
    pub fn unresolved_count(&self) -> usize {
        self.unresolved.len()
    }

    /// Whether the corpus is foundation-coherent: no detected
    /// conflicts. Unresolved citations don't count as conflicts —
    /// they're observability data, not logical contradictions.
    pub fn is_coherent(&self) -> bool {
        self.conflicts.is_empty()
    }

    /// Whether the corpus has zero citations across the board (an
    /// empty manifest).
    pub fn is_empty(&self) -> bool {
        self.by_foundation.is_empty() && self.unresolved.is_empty()
    }
}

/// Detect every pairwise incompatibility among the distinct
/// foundations present. Returns at most `n*(n-1)/2` conflicts
/// where `n = by_foundation.len()`. Order is canonical
/// (lexicographic by tag) so reports are reproducible.
fn detect_pairwise_conflicts(
    by_foundation: &BTreeMap<FoundationProfile, usize>,
) -> Vec<FoundationConflict> {
    let foundations: Vec<FoundationProfile> = by_foundation.keys().copied().collect();
    let mut conflicts = Vec::new();
    for i in 0..foundations.len() {
        for j in (i + 1)..foundations.len() {
            let left = foundations[i];
            let right = foundations[j];
            if left.conflicts_with(right) {
                conflicts.push(FoundationConflict {
                    left,
                    right,
                    reason: conflict_reason(left, right),
                });
            }
        }
    }
    conflicts
}

/// Human-readable explanation for a detected conflict. Currently
/// only UIP + univalence is detectable; the match is exhaustive on
/// the conflict cases enumerated by [`FoundationProfile::conflicts_with`].
fn conflict_reason(a: FoundationProfile, b: FoundationProfile) -> String {
    let (uip, uni) = if a.assumes_uip() && b.assumes_univalence() {
        (a, b)
    } else if b.assumes_uip() && a.assumes_univalence() {
        (b, a)
    } else {
        return format!(
            "incompatible foundations: {} and {}",
            a.tag(),
            b.tag(),
        );
    };
    format!(
        "{} assumes UIP; {} assumes univalence — UIP and univalence are \
         logically incompatible (Hofmann-Streicher 1996, Voevodsky 2010)",
        uip.tag(),
        uni.tag(),
    )
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

    #[test]
    fn from_framework_tag_bridges_existing_corpus() {
        // These tags appear in the actual core/math/ corpus
        // (`@framework(hott, "...")`, `@framework(cubical, "...")`,
        // `@framework(msfs, "...")`). The bridge maps each to its
        // foundation.
        assert_eq!(
            FoundationProfile::from_framework_tag("hott"),
            Some(FoundationProfile::Hott),
        );
        assert_eq!(
            FoundationProfile::from_framework_tag("cubical"),
            Some(FoundationProfile::Cubical),
        );
        assert_eq!(
            FoundationProfile::from_framework_tag("zfc"),
            Some(FoundationProfile::Zfc),
        );
        assert_eq!(
            FoundationProfile::from_framework_tag("msfs"),
            Some(FoundationProfile::ZfcTwoInaccessibles),
            "MSFS uses ZFC + 2 inaccessibles per the corpus README",
        );
        assert_eq!(
            FoundationProfile::from_framework_tag("htt"),
            Some(FoundationProfile::ZfcOneInaccessible),
            "Lurie HTT requires ZFC + 1 inaccessible",
        );
    }

    #[test]
    fn from_framework_tag_aliases() {
        // UIP tag → MlttUip.
        assert_eq!(
            FoundationProfile::from_framework_tag("uip"),
            Some(FoundationProfile::MlttUip),
        );
        // Coq tag → CIC (Coq's kernel logic).
        assert_eq!(
            FoundationProfile::from_framework_tag("coq"),
            Some(FoundationProfile::Cic),
        );
    }

    #[test]
    fn from_framework_tag_returns_none_for_framework_names() {
        // Framework names (specific corpora WITHIN a foundation) are
        // not foundations themselves — they cite results in the
        // foundation's literature. Return None so consumers
        // dispatch through a separate framework→foundation map.
        assert!(FoundationProfile::from_framework_tag("lurie_htt").is_none());
        assert!(FoundationProfile::from_framework_tag("schreiber_dcct").is_none());
        assert!(FoundationProfile::from_framework_tag("baez_dolan").is_none());
        assert!(FoundationProfile::from_framework_tag("connes_reconstruction").is_none());
    }

    #[test]
    fn from_framework_tag_unknown_returns_none() {
        assert!(FoundationProfile::from_framework_tag("").is_none());
        assert!(FoundationProfile::from_framework_tag("some_garbage_tag").is_none());
        assert!(FoundationProfile::from_framework_tag("ZFC").is_none(), "case-sensitive");
    }

    #[test]
    fn from_known_framework_msfs_family_is_zfc_two_inaccessibles() {
        // The MSFS family of frameworks all share Verum's default
        // meta-theory: ZFC + 2 Grothendieck inaccessibles.
        for framework in [
            "msfs",
            "diakrisis",
            "connes_reconstruction",
            "baez_dolan",
            "schreiber_dcct",
            "petz_classification",
            "adamek_rosicky",
            "lair_makkai_pare",
            "lambek_scott",
        ] {
            assert_eq!(
                FoundationProfile::from_known_framework(framework),
                Some(FoundationProfile::ZfcTwoInaccessibles),
                "framework {:?} should resolve to ZFC + 2 inaccessibles",
                framework,
            );
        }
    }

    #[test]
    fn from_known_framework_lurie_htt_is_zfc_one_inaccessible() {
        assert_eq!(
            FoundationProfile::from_known_framework("lurie_htt"),
            Some(FoundationProfile::ZfcOneInaccessible),
        );
    }

    #[test]
    fn from_known_framework_cic_external_proof_assistants() {
        // Coq + Lean 4 + Lean's mathlib4 all share the CIC family.
        for framework in ["coq_stdlib", "lean4_stdlib", "mathlib4"] {
            assert_eq!(
                FoundationProfile::from_known_framework(framework),
                Some(FoundationProfile::Cic),
                "framework {:?} should resolve to CIC",
                framework,
            );
        }
    }

    #[test]
    fn from_known_framework_verum_self_citation() {
        // The `@framework(verum, "...")` self-citation marks the
        // trust dependency on Verum's own stdlib results — Verum's
        // default foundation is ZFC + 2 inaccessibles.
        assert_eq!(
            FoundationProfile::from_known_framework("verum"),
            Some(FoundationProfile::ZfcTwoInaccessibles),
        );
    }

    #[test]
    fn from_known_framework_pure_zfc_corpora() {
        for framework in [
            "arnold_catastrophe",
            "bounded_arithmetic_i_delta_0",
            "bounded_arithmetic_s_2_1",
            "bounded_arithmetic_v_0",
            "bounded_arithmetic_v_1",
            "bounded_arithmetic_v_np",
            "bounded_arithmetic_v_ph",
        ] {
            assert_eq!(
                FoundationProfile::from_known_framework(framework),
                Some(FoundationProfile::Zfc),
                "framework {:?} should resolve to plain ZFC",
                framework,
            );
        }
    }

    #[test]
    fn from_known_framework_unknown_returns_none() {
        // Domain-specific or unrecognised tags are not foundations.
        assert!(FoundationProfile::from_known_framework("owl2_fs").is_none());
        assert!(FoundationProfile::from_known_framework("").is_none());
        assert!(FoundationProfile::from_known_framework("garbage").is_none());
    }

    #[test]
    fn from_known_framework_does_not_overlap_foundation_tags() {
        // Foundation-level tags belong to `from_framework_tag`, not
        // `from_known_framework`. Keep the boundary clean so
        // `resolve_citation` always picks the foundation-tag bridge
        // first when both could match.
        assert!(FoundationProfile::from_known_framework("hott").is_none());
        assert!(FoundationProfile::from_known_framework("cubical").is_none());
        assert!(FoundationProfile::from_known_framework("zfc").is_none());
        assert!(FoundationProfile::from_known_framework("htt").is_none());
        assert!(FoundationProfile::from_known_framework("cic").is_none());
    }

    #[test]
    fn resolve_citation_combines_both_bridges() {
        // Foundation-level tags resolve via `from_framework_tag`.
        assert_eq!(
            FoundationProfile::resolve_citation("hott"),
            Some(FoundationProfile::Hott),
        );
        // Framework-level tags resolve via `from_known_framework`.
        assert_eq!(
            FoundationProfile::resolve_citation("lurie_htt"),
            Some(FoundationProfile::ZfcOneInaccessible),
        );
        assert_eq!(
            FoundationProfile::resolve_citation("diakrisis"),
            Some(FoundationProfile::ZfcTwoInaccessibles),
        );
        // Unknown tags return None.
        assert!(FoundationProfile::resolve_citation("garbage").is_none());
        assert!(FoundationProfile::resolve_citation("owl2_fs").is_none());
    }

    fn cite(framework: &str, citation: &str, decl: &str, kind: &str) -> FrameworkCitation {
        FrameworkCitation {
            decl_name: decl.to_string(),
            decl_kind: kind.to_string(),
            framework: framework.to_string(),
            citation: citation.to_string(),
        }
    }

    #[test]
    fn distribution_partitions_resolved_and_unresolved() {
        let citations = vec![
            cite("hott", "1.2.3", "uni_lemma", "lemma"),
            cite("msfs", "5.1", "afnt_alpha", "theorem"),
            cite("lurie_htt", "5.5", "topos_lemma", "lemma"),
            cite("garbage_unknown", "—", "weird_thm", "theorem"),
        ];
        let dist = FoundationDistribution::from_citations(&citations);
        assert_eq!(dist.resolved_count(), 3);
        assert_eq!(dist.unresolved_count(), 1);
        assert_eq!(
            dist.by_foundation[&FoundationProfile::Hott],
            1,
        );
        assert_eq!(
            dist.by_foundation[&FoundationProfile::ZfcTwoInaccessibles],
            1,
        );
        assert_eq!(
            dist.by_foundation[&FoundationProfile::ZfcOneInaccessible],
            1,
        );
        assert_eq!(dist.unresolved[0].framework, "garbage_unknown");
    }

    #[test]
    fn distribution_aggregates_repeated_frameworks() {
        let citations = vec![
            cite("msfs", "1", "a", "theorem"),
            cite("msfs", "2", "b", "theorem"),
            cite("diakrisis", "3", "c", "theorem"),
        ];
        let dist = FoundationDistribution::from_citations(&citations);
        // msfs (2) + diakrisis (1) — both ZfcTwoInaccessibles → 3.
        assert_eq!(
            dist.by_foundation[&FoundationProfile::ZfcTwoInaccessibles],
            3,
        );
        assert_eq!(dist.foundations().len(), 1);
    }

    #[test]
    fn distribution_detects_uip_univalence_conflict() {
        let citations = vec![
            cite("hott", "univalence", "uni_thm", "theorem"),
            cite("uip", "uniqueness", "uip_thm", "theorem"),
        ];
        let dist = FoundationDistribution::from_citations(&citations);
        assert!(!dist.is_coherent());
        assert_eq!(dist.conflicts.len(), 1);
        let c = &dist.conflicts[0];
        assert!(c.reason.contains("UIP"));
        assert!(c.reason.contains("univalence"));
        // One of left/right is MlttUip, the other is Hott.
        assert!(
            (c.left == FoundationProfile::MlttUip && c.right == FoundationProfile::Hott)
                || (c.left == FoundationProfile::Hott
                    && c.right == FoundationProfile::MlttUip),
        );
    }

    #[test]
    fn distribution_no_conflict_when_foundations_compatible() {
        let citations = vec![
            cite("hott", "x", "a", "theorem"),
            cite("cubical", "y", "b", "theorem"),
            cite("msfs", "z", "c", "theorem"),
        ];
        let dist = FoundationDistribution::from_citations(&citations);
        assert!(dist.is_coherent());
    }

    #[test]
    fn distribution_empty_manifest_has_no_state() {
        let dist = FoundationDistribution::from_citations(&[]);
        assert_eq!(dist.resolved_count(), 0);
        assert_eq!(dist.unresolved_count(), 0);
        assert!(dist.is_coherent());
        assert!(dist.is_empty());
    }

    #[test]
    fn distribution_serde_round_trip() {
        let citations = vec![
            cite("hott", "1", "a", "theorem"),
            cite("msfs", "2", "b", "theorem"),
            cite("garbage", "3", "c", "theorem"),
        ];
        let dist = FoundationDistribution::from_citations(&citations);
        let json = serde_json::to_string(&dist).unwrap();
        let restored: FoundationDistribution = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, dist);
    }

    #[test]
    fn distribution_from_manifest_matches_from_citations() {
        let citations = vec![cite("hott", "1", "a", "theorem")];
        let mut manifest = FrameworkCitationManifest::empty();
        for c in &citations {
            *manifest
                .by_framework
                .entry(c.framework.clone())
                .or_insert(0) += 1;
            manifest.rows.push(c.clone());
        }
        assert_eq!(
            FoundationDistribution::from_manifest(&manifest),
            FoundationDistribution::from_citations(&citations),
        );
    }

    #[test]
    fn resolve_citation_msfs_tag_prefers_foundation_bridge() {
        // The string `"msfs"` is recognised by BOTH bridges (the
        // foundation-tag bridge as ZfcTwoInaccessibles, the framework
        // bridge as ZfcTwoInaccessibles). `resolve_citation` picks
        // the foundation-tag bridge first; both must agree.
        let from_tag = FoundationProfile::from_framework_tag("msfs");
        let from_known = FoundationProfile::from_known_framework("msfs");
        assert_eq!(from_tag, Some(FoundationProfile::ZfcTwoInaccessibles));
        assert_eq!(from_known, Some(FoundationProfile::ZfcTwoInaccessibles));
        assert_eq!(
            FoundationProfile::resolve_citation("msfs"),
            Some(FoundationProfile::ZfcTwoInaccessibles),
        );
    }
}
