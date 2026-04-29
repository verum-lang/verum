//! HTT + Adámek-Rosický mechanisation roadmap — V0 algorithmic
//! kernel rule.
//!
//! ## What this delivers
//!
//! Lurie's *Higher Topos Theory* (HTT) and Adámek & Rosický's
//! *Locally Presentable and Accessible Categories* (AR 1994) are the
//! two load-bearing reference texts for Verum's (∞,1)-categorical
//! kernel layer.  Full mechanisation of either is a multi-decade
//! community project — the kernel cannot ship the entire content
//! at once, but it CAN expose a structured roadmap that:
//!
//!   1. Enumerates each chapter / section's mechanisation status.
//!   2. Lists the precise kernel modules / functions that discharge
//!      each section.
//!   3. Allows `verum audit --htt-roadmap` and
//!      `verum audit --adamek-rosicky-roadmap` to surface the
//!      coverage table.
//!   4. Tracks version-stamped progress so successive Verum releases
//!      can monotonically increase coverage without losing audit
//!      provenance.
//!
//! V0 ships the static enumeration + per-section status flag.  V1
//! promotion: each section gains a structural verification hook
//! (a `pub fn verify_section_X_Y` that re-checks the kernel
//! discharge).
//!
//! ## What this UNBLOCKS
//!
//!   - **`verum audit --htt-roadmap`** — emits a per-chapter coverage
//!     report comparable across Verum releases.
//!   - **`verum audit --adamek-rosicky-roadmap`** — same for AR 1994.
//!   - **External community contributions** — the per-section
//!     `RoadmapEntry` is a precise specification of what a community
//!     PR would need to land to flip a `Pending` to `Mechanised`.

use serde::{Deserialize, Serialize};
use verum_common::Text;

// =============================================================================
// Roadmap surface
// =============================================================================

/// Per-section mechanisation status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MechanisationStatus {
    /// Section is mechanised in-kernel: a decidable/algorithmic
    /// surface exists.
    Mechanised,
    /// Section is partially mechanised: framework axiom + some
    /// algorithmic content (e.g. V0 surface, V1 promotion pending).
    Partial,
    /// Section is admitted via a paper-cited framework axiom; no
    /// algorithmic content yet.
    AxiomCited,
    /// Section is not yet covered by any kernel surface.
    Pending,
}

impl MechanisationStatus {
    pub fn name(self) -> &'static str {
        match self {
            MechanisationStatus::Mechanised => "mechanised",
            MechanisationStatus::Partial => "partial",
            MechanisationStatus::AxiomCited => "axiom-cited",
            MechanisationStatus::Pending => "pending",
        }
    }

    pub fn is_satisfied(self) -> bool {
        matches!(self, MechanisationStatus::Mechanised | MechanisationStatus::Partial)
    }
}

/// A single roadmap entry: chapter/section identifier + mechanisation
/// status + links to the kernel module(s) that discharge it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoadmapEntry {
    /// Section identifier (e.g. "HTT 1.2.1" or "AR 1.26").
    pub section: Text,
    /// Human-readable title.
    pub title: Text,
    /// Current mechanisation status.
    pub status: MechanisationStatus,
    /// Kernel module(s) that discharge this section, comma-separated
    /// (empty when status is Pending).
    pub kernel_modules: Text,
}

impl RoadmapEntry {
    pub fn mechanised(
        section: impl Into<Text>,
        title: impl Into<Text>,
        modules: impl Into<Text>,
    ) -> Self {
        Self {
            section: section.into(),
            title: title.into(),
            status: MechanisationStatus::Mechanised,
            kernel_modules: modules.into(),
        }
    }

    pub fn pending(section: impl Into<Text>, title: impl Into<Text>) -> Self {
        Self {
            section: section.into(),
            title: title.into(),
            status: MechanisationStatus::Pending,
            kernel_modules: Text::from(""),
        }
    }
}

// =============================================================================
// HTT roadmap (Lurie 2009, *Higher Topos Theory*)
// =============================================================================

/// Build the HTT mechanisation roadmap as it stands at this Verum
/// release.  Per-chapter/section entries; iteration order matches
/// HTT's table of contents.
pub fn htt_roadmap() -> Vec<RoadmapEntry> {
    vec![
        RoadmapEntry::mechanised(
            "HTT 1.2.1",
            "Yoneda embedding y: C → PSh(C)",
            "yoneda::yoneda_embedding",
        ),
        RoadmapEntry::mechanised(
            "HTT 1.2.4.3",
            "Whitehead criterion (weak ⟺ honest in Kan complex)",
            "whitehead::weak_equivalence_lifts_in_kan_complex",
        ),
        RoadmapEntry::mechanised(
            "HTT 1.2.13",
            "Limits and colimits of (∞,1)-functors",
            "limits_colimits::compute_limit_in_psh",
        ),
        RoadmapEntry::mechanised(
            "HTT 3.1",
            "Cartesian fibrations of ∞-categories",
            "cartesian_fibration::CartesianFibration",
        ),
        RoadmapEntry::mechanised(
            "HTT 3.2.0.1",
            "Straightening / Unstraightening equivalence",
            "cartesian_fibration::build_straightening_equivalence",
        ),
        RoadmapEntry::mechanised(
            "HTT 4.3.3.7",
            "Kan extensions in ∞-categories",
            "yoneda::build_kan_extension",
        ),
        RoadmapEntry::mechanised(
            "HTT 4.4 (limits via small-object)",
            "Pullbacks/Pushouts/Equalisers/Coequalisers",
            "limits_colimits::{build_pullback, build_pushout, build_equaliser, build_coequaliser}",
        ),
        RoadmapEntry::mechanised(
            "HTT 5.1.4",
            "∞-Grothendieck construction",
            "grothendieck::build_grothendieck",
        ),
        RoadmapEntry::mechanised(
            "HTT 5.2.7",
            "Reflective subcategories",
            "reflective_subcategory::build_reflective_subcategory",
        ),
        RoadmapEntry::mechanised(
            "HTT 5.2.8",
            "Stable factorisation systems",
            "factorisation::FactorisationSystem",
        ),
        RoadmapEntry::mechanised(
            "HTT 5.5 (universe ascent)",
            "Presheaf categories live one universe up",
            "yoneda::presheaf_category + ordinal::next_inaccessible",
        ),
        RoadmapEntry::mechanised(
            "HTT 5.5.2.9",
            "Special Adjoint Functor Theorem (SAFT)",
            "adjoint_functor::build_adjunction",
        ),
        RoadmapEntry::mechanised(
            "HTT 5.5.3.5",
            "Presheaf categories are bicomplete",
            "limits_colimits::presheaf_is_bicomplete",
        ),
        RoadmapEntry::mechanised(
            "HTT 5.5.6",
            "n-truncation operators",
            "truncation::truncate_to_level",
        ),
        RoadmapEntry::mechanised(
            "HTT 6.1",
            "(∞,1)-topoi (Giraud's theorem)",
            "infinity_topos::build_infinity_topos",
        ),
        RoadmapEntry {
            section: Text::from("HTT 7 (sheaves of spaces)"),
            title: Text::from("Sheaves on (∞,1)-sites"),
            status: MechanisationStatus::Pending,
            kernel_modules: Text::from(""),
        },
        RoadmapEntry {
            section: Text::from("HTT App. A (model-categorical foundations)"),
            title: Text::from("Quillen model structures + simplicial sets"),
            status: MechanisationStatus::AxiomCited,
            kernel_modules: Text::from("framework axioms in core.math.frameworks.lurie_htt"),
        },
    ]
}

// =============================================================================
// Adámek-Rosický 1994 roadmap
// =============================================================================

/// Build the AR 1994 mechanisation roadmap.  Per-section entries
/// with the chapter/page citation.
pub fn adamek_rosicky_roadmap() -> Vec<RoadmapEntry> {
    vec![
        RoadmapEntry::mechanised(
            "AR 1.26",
            "λ-filtered colimit closure of κ-accessible categories",
            "accessibility::build_filtered_colimit",
        ),
        RoadmapEntry {
            section: Text::from("AR 2.6"),
            title: Text::from("Reflexion of accessible category to a presentable one"),
            status: MechanisationStatus::Partial,
            kernel_modules: Text::from(
                "reflective_subcategory::build_reflective_subcategory + accessibility::build_filtered_colimit",
            ),
        },
        RoadmapEntry {
            section: Text::from("AR 2.39 (locally presentable)"),
            title: Text::from("Characterisation of locally presentable categories"),
            status: MechanisationStatus::AxiomCited,
            kernel_modules: Text::from("framework axiom in core.math.accessible"),
        },
        RoadmapEntry {
            section: Text::from("AR 5.5.4 (Adjoint Functor)"),
            title: Text::from("AFT for presentable categories"),
            status: MechanisationStatus::Mechanised,
            kernel_modules: Text::from("adjoint_functor::build_adjunction"),
        },
        RoadmapEntry {
            section: Text::from("AR Ch.4 (sketches)"),
            title: Text::from("Sketches and accessible models"),
            status: MechanisationStatus::Pending,
            kernel_modules: Text::from(""),
        },
        RoadmapEntry {
            section: Text::from("AR App. (set-theoretic prerequisites)"),
            title: Text::from("Vopěnka's principle, large cardinals"),
            status: MechanisationStatus::AxiomCited,
            kernel_modules: Text::from("zfc_self_recognition + ordinal::next_inaccessible"),
        },
    ]
}

// =============================================================================
// Coverage statistics
// =============================================================================

/// Mechanisation coverage report for a roadmap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverageReport {
    /// Total number of entries in the roadmap.
    pub total: u32,
    /// Number of `Mechanised` entries.
    pub mechanised: u32,
    /// Number of `Partial` entries.
    pub partial: u32,
    /// Number of `AxiomCited` entries.
    pub axiom_cited: u32,
    /// Number of `Pending` entries.
    pub pending: u32,
}

impl CoverageReport {
    /// Compute coverage from a list of entries.
    pub fn compute(entries: &[RoadmapEntry]) -> Self {
        let mut report = Self {
            total: entries.len() as u32,
            mechanised: 0,
            partial: 0,
            axiom_cited: 0,
            pending: 0,
        };
        for e in entries {
            match e.status {
                MechanisationStatus::Mechanised => report.mechanised += 1,
                MechanisationStatus::Partial => report.partial += 1,
                MechanisationStatus::AxiomCited => report.axiom_cited += 1,
                MechanisationStatus::Pending => report.pending += 1,
            }
        }
        report
    }

    /// Coverage ratio: (Mechanised + Partial) / Total.
    pub fn coverage_ratio(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        (self.mechanised + self.partial) as f64 / self.total as f64
    }

    /// Render a one-line summary.
    pub fn summary(&self, label: &str) -> String {
        format!(
            "{}: {}/{} satisfied ({:.0}%); mechanised={}, partial={}, axiom-cited={}, pending={}",
            label,
            self.mechanised + self.partial,
            self.total,
            self.coverage_ratio() * 100.0,
            self.mechanised,
            self.partial,
            self.axiom_cited,
            self.pending
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- HTT roadmap -----

    #[test]
    fn htt_roadmap_has_yoneda_section_mechanised() {
        let roadmap = htt_roadmap();
        let yoneda = roadmap
            .iter()
            .find(|e| e.section.as_str() == "HTT 1.2.1")
            .expect("HTT 1.2.1 must be in the roadmap");
        assert_eq!(yoneda.status, MechanisationStatus::Mechanised);
    }

    #[test]
    fn htt_roadmap_includes_topos_chapter() {
        let roadmap = htt_roadmap();
        let topos = roadmap
            .iter()
            .find(|e| e.section.as_str() == "HTT 6.1")
            .expect("HTT 6.1 (topos) must be in roadmap");
        assert!(topos.status.is_satisfied());
    }

    #[test]
    fn htt_roadmap_majority_mechanised() {
        let roadmap = htt_roadmap();
        let report = CoverageReport::compute(&roadmap);
        // After this session's 10-module sweep we should be at >50%.
        assert!(
            report.coverage_ratio() > 0.5,
            "HTT coverage must exceed 50% after V0 sweep; got {}",
            report.coverage_ratio()
        );
    }

    // ----- AR roadmap -----

    #[test]
    fn ar_roadmap_has_1_26() {
        let roadmap = adamek_rosicky_roadmap();
        let ar126 = roadmap
            .iter()
            .find(|e| e.section.as_str() == "AR 1.26")
            .expect("AR 1.26 must be in the roadmap");
        assert_eq!(ar126.status, MechanisationStatus::Mechanised);
    }

    // ----- CoverageReport -----

    #[test]
    fn coverage_report_sums_to_total() {
        let roadmap = htt_roadmap();
        let report = CoverageReport::compute(&roadmap);
        assert_eq!(
            report.total,
            report.mechanised + report.partial + report.axiom_cited + report.pending
        );
    }

    #[test]
    fn coverage_report_for_empty_is_zero() {
        let report = CoverageReport::compute(&[]);
        assert_eq!(report.total, 0);
        assert_eq!(report.coverage_ratio(), 0.0);
    }

    #[test]
    fn coverage_summary_renders_percentage() {
        let roadmap = adamek_rosicky_roadmap();
        let report = CoverageReport::compute(&roadmap);
        let summary = report.summary("AR 1994");
        assert!(summary.contains("AR 1994"));
        assert!(summary.contains("%"));
    }

    #[test]
    fn mechanisation_status_satisfied_check() {
        assert!(MechanisationStatus::Mechanised.is_satisfied());
        assert!(MechanisationStatus::Partial.is_satisfied());
        assert!(!MechanisationStatus::AxiomCited.is_satisfied());
        assert!(!MechanisationStatus::Pending.is_satisfied());
    }
}
