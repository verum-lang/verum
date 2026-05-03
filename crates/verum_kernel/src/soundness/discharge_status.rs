//! Unified soundness-bookkeeping discharge status.
//!
//! Single canonical type used by every Verum manifest that tracks
//! "is a soundness obligation discharged, admitted with IOU, or not
//! yet attested?". Pre-this-module two parallel enums lived:
//!
//!   * `KernelV0Status` (in `kernel_v0_manifest`) — `Proved | Admitted`,
//!     with the IOU citation in a separate `KernelV0Rule.iou_citation`
//!     field.
//!   * `AttestationStatus` (in `codegen_attestation`) — `Discharged |
//!     AdmittedWithIou { iou } | NotYetAttested`, with the IOU
//!     embedded in the enum variant.
//!
//! The two patterns covered the same architectural concept — soundness
//! discharge tracking — through different shapes. `DischargeStatus`
//! (this module) is the single canonical form: IOU embedded in the
//! enum variant (cleaner data model), three states (Discharged,
//! AdmittedWithIou, NotYetAttested) covering both manifests.
//!
//! # Architectural role
//!
//! Both manifests now use this type uniformly. Audit gates
//! (`verum audit --kernel-v0-roster` + `verum audit
//! --codegen-attestation`) consume the same pattern matches; helper
//! counts (`is_discharged` / `is_admitted` / `is_pending`) are shared.
//! Adding a new manifest that tracks soundness discharge — e.g. a
//! per-anti-pattern soundness manifest, or a per-bridge attestation
//! manifest — reuses this type without re-invention.
//!
//! # Soundness chronicle integration
//!
//! Every audit JSON output that surfaces this enum uses the canonical
//! serde representation:
//!
//! ```text
//! discharged                ← DischargeStatus::Discharged
//! admitted_with_iou         ← DischargeStatus::AdmittedWithIou { iou }
//! not_yet_attested          ← DischargeStatus::NotYetAttested
//! ```
//!
//! Schema-versioned consumers (e.g. CI dashboards) can rely on these
//! tags being stable across releases.

use serde::{Deserialize, Serialize};

/// Discharge status for any soundness obligation Verum tracks in a
/// manifest.
///
/// **Four canonical states:**
///
/// * [`Self::Discharged`] — the obligation has a kernel-checked
///   structural proof. No IOU; the manifest entry's `proof_obligation`
///   field is the citation for the discharged proof.
/// * [`Self::DischargedByFramework`] — the obligation is admitted via
///   a *vetted upstream proof* in a registered framework
///   (mathlib4 / coq_stdlib / zfc / lean4_stdlib / ...). Distinct
///   from `AdmittedWithIou` because the IOU has been *resolved* by
///   citation rather than left open. Carries a structured citation
///   triple (`lemma_path`, `framework`, `citation`) that audit gates
///   walk uniformly. This is the canonical mid-state between an
///   open IOU and a fully kernel-checked proof.
/// * [`Self::AdmittedWithIou`] — the obligation is admitted with a
///   structural-property IOU. The IOU payload names the missing
///   structural lemma (e.g. "substitution-lemma (Barendregt 1984)").
///   This is the CompCert `Lemma X. Admitted.` shape — honest about
///   the gap. Mature manifests promote AdmittedWithIou →
///   DischargedByFramework once a vetted upstream citation lands.
/// * [`Self::NotYetAttested`] — the obligation has not yet been
///   attested at all (neither discharged nor structurally admitted).
///   The pre-attestation surface — "trusted by code review only".
///   Manifests that want to forbid this state simply never construct
///   it; the kernel_v0 manifest does so for example, since every
///   bootstrap rule has at least an admit citation.
///
/// **Lifecycle:**
///
/// ```text
/// NotYetAttested → AdmittedWithIou → DischargedByFramework → Discharged
/// ```
///
/// Each transition reduces the trust-extension surface by one level.
/// `Discharged` is the strongest claim (kernel-checked); the three
/// preceding states are honest about increasing levels of structured
/// admission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DischargeStatus {
    /// Obligation has a kernel-checked structural proof.
    Discharged,
    /// Obligation admitted via a vetted upstream proof in a
    /// registered framework (mathlib4 / coq_stdlib / zfc / etc).
    /// Audit-acceptable as an L4 trust extension because the
    /// citation pins a specific upstream artefact a reviewer can
    /// independently verify.
    DischargedByFramework {
        /// Path to the discharge stub in
        /// `core/verify/kernel_v0/lemmas/` (or analogous directory
        /// for other manifests). Example:
        /// `core.verify.kernel_v0.lemmas.beta.church_rosser_confluence`.
        lemma_path: String,
        /// Upstream framework name. Examples: `"mathlib4"`,
        /// `"coq_stdlib"`, `"lean4_stdlib"`, `"zfc"`,
        /// `"barendregt_1984"`, `"hofmann_streicher_1996"`.
        framework: String,
        /// Concrete citation string. Examples:
        /// `"Mathlib.Computability.Lambda.ChurchRosser"`,
        /// `"Barendregt 1984 §3.2.8 Church-Rosser"`,
        /// `"Hofmann-Streicher 1996 §4.3 funext"`.
        citation: String,
    },
    /// Obligation admitted with a structural-property IOU. The
    /// payload names the missing structural lemma verbatim.
    AdmittedWithIou {
        /// Concrete IOU naming the missing structural lemma.
        /// Preserved verbatim into audit reports.
        iou: String,
    },
    /// Obligation not yet attested. Pre-attestation surface for new
    /// manifests; mature manifests typically eliminate this state by
    /// always citing at least an IOU.
    NotYetAttested,
}

impl DischargeStatus {
    /// Stable diagnostic tag — matches the serde representation
    /// modulo the IOU / framework-citation payloads.
    pub fn tag(&self) -> &'static str {
        match self {
            DischargeStatus::Discharged => "discharged",
            DischargeStatus::DischargedByFramework { .. } => "discharged_by_framework",
            DischargeStatus::AdmittedWithIou { .. } => "admitted_with_iou",
            DischargeStatus::NotYetAttested => "not_yet_attested",
        }
    }

    /// Human-readable display name.
    pub fn display_name(&self) -> &'static str {
        match self {
            DischargeStatus::Discharged => "Discharged",
            DischargeStatus::DischargedByFramework { .. } => "Discharged by framework",
            DischargeStatus::AdmittedWithIou { .. } => "Admitted with IOU",
            DischargeStatus::NotYetAttested => "Not yet attested",
        }
    }

    /// True iff the obligation carries a kernel-discharged proof.
    /// **Does NOT include** `DischargedByFramework` — that's a
    /// distinct trust-extension level. Use [`Self::is_audit_clean`]
    /// for the predicate "this obligation is at least as strong as
    /// an L4-acceptable framework discharge".
    pub fn is_discharged(&self) -> bool {
        matches!(self, DischargeStatus::Discharged)
    }

    /// True iff the obligation carries a vetted upstream-framework
    /// citation that resolves the IOU.
    pub fn is_discharged_by_framework(&self) -> bool {
        matches!(self, DischargeStatus::DischargedByFramework { .. })
    }

    /// True iff the obligation carries a structural-IOU admit.
    pub fn is_admitted(&self) -> bool {
        matches!(self, DischargeStatus::AdmittedWithIou { .. })
    }

    /// True iff the obligation has not been attested at all.
    pub fn is_pending(&self) -> bool {
        matches!(self, DischargeStatus::NotYetAttested)
    }

    /// True iff the obligation is *audit-clean* — either
    /// kernel-discharged structurally or discharged-by-framework
    /// with a vetted upstream citation. Both states are
    /// L4-acceptable; only `AdmittedWithIou` and `NotYetAttested`
    /// fail the audit-clean check.
    pub fn is_audit_clean(&self) -> bool {
        self.is_discharged() || self.is_discharged_by_framework()
    }

    /// Borrow the IOU payload when the status is
    /// `AdmittedWithIou`. `None` for the other variants.
    pub fn iou(&self) -> Option<&str> {
        match self {
            DischargeStatus::AdmittedWithIou { iou } => Some(iou.as_str()),
            _ => None,
        }
    }

    /// Borrow the framework citation when the status is
    /// `DischargedByFramework`. Returns the structured citation
    /// triple `(lemma_path, framework, citation)` for uniform audit
    /// rendering.
    pub fn framework_citation(&self) -> Option<(&str, &str, &str)> {
        match self {
            DischargeStatus::DischargedByFramework {
                lemma_path,
                framework,
                citation,
            } => Some((lemma_path.as_str(), framework.as_str(), citation.as_str())),
            _ => None,
        }
    }
}

impl std::fmt::Display for DischargeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_matches_serde() {
        assert_eq!(DischargeStatus::Discharged.tag(), "discharged");
        assert_eq!(
            DischargeStatus::DischargedByFramework {
                lemma_path: "p".to_string(),
                framework: "f".to_string(),
                citation: "c".to_string(),
            }
            .tag(),
            "discharged_by_framework"
        );
        assert_eq!(
            DischargeStatus::AdmittedWithIou {
                iou: "x".to_string()
            }
            .tag(),
            "admitted_with_iou"
        );
        assert_eq!(DischargeStatus::NotYetAttested.tag(), "not_yet_attested");
    }

    #[test]
    fn classification_predicates() {
        let d = DischargeStatus::Discharged;
        assert!(d.is_discharged() && !d.is_admitted() && !d.is_pending());
        assert!(d.is_audit_clean());

        let f = DischargeStatus::DischargedByFramework {
            lemma_path: "core.verify.kernel_v0.lemmas.beta.church_rosser_confluence".to_string(),
            framework: "mathlib4".to_string(),
            citation: "Mathlib.Computability.Lambda.ChurchRosser".to_string(),
        };
        assert!(
            f.is_discharged_by_framework() && !f.is_discharged() && !f.is_admitted() && !f.is_pending()
        );
        assert!(f.is_audit_clean());
        assert_eq!(
            f.framework_citation(),
            Some((
                "core.verify.kernel_v0.lemmas.beta.church_rosser_confluence",
                "mathlib4",
                "Mathlib.Computability.Lambda.ChurchRosser",
            ))
        );

        let a = DischargeStatus::AdmittedWithIou {
            iou: "Newman's lemma".to_string(),
        };
        assert!(a.is_admitted() && !a.is_discharged() && !a.is_pending());
        assert!(!a.is_audit_clean());
        assert_eq!(a.iou(), Some("Newman's lemma"));

        let n = DischargeStatus::NotYetAttested;
        assert!(n.is_pending() && !n.is_discharged() && !n.is_admitted());
        assert!(!n.is_audit_clean());
        assert_eq!(n.iou(), None);
    }

    #[test]
    fn serde_roundtrip() {
        let states = [
            DischargeStatus::Discharged,
            DischargeStatus::DischargedByFramework {
                lemma_path: "lemma".to_string(),
                framework: "framework".to_string(),
                citation: "citation".to_string(),
            },
            DischargeStatus::AdmittedWithIou {
                iou: "test IOU".to_string(),
            },
            DischargeStatus::NotYetAttested,
        ];
        for s in &states {
            let json = serde_json::to_string(s).unwrap();
            let back: DischargeStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(s, &back);
        }
    }
}
