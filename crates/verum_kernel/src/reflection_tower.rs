//! Legacy reflection-structure diagnostics.
//!
//! The stage flags, finite index samples, universe budgets and bibliography in
//! this module are inspectable data. They neither implement the cited category
//! theory nor provide proofs of reflection, soundness or stabilization. A finite
//! stability walk cannot establish a claim over every universe level.
//!
//! `proof_verdict` is therefore always `StructuralOnly`; positive structural
//! observations remain available separately. Mathematical certification requires
//! an exact formal statement and native proof replay outside this diagnostic API.
//! Historical citation labels below are provenance, not verified theorem status.

use crate::intrinsic_dispatch::{IntrinsicValue, dispatch_intrinsic};
use crate::zfc_self_recognition::{KernelRuleId, kernel_meta_soundness_holds, required_meta_theory};

// =============================================================================
// MSFS citation — bound to the corpus
// =============================================================================

/// Historical bibliography labels retained for diagnostic provenance.
/// No path or theorem number is resolved to a verified proof by this module.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MsfsCitation {
    /// Declared per-rule footprint metadata.
    BaseFootprint,
    /// Historical claim labelled MSFS Theorem 9.6 (Meta-classification stabilisation):
    /// `𝔐^(Cls·k) ≃_2 𝔐^(Cls)` for every `k ≥ 1` (theory-level
    /// idempotence; set-theoretic universe-ascent).
    MsfsTheorem_9_6_MetaStabilisation,
    /// Historical claim labelled MSFS Theorem 8.2 (Reflective tower boundedness):
    /// `Con(tower over S) = Con(S) + κ_inacc` — exactly one
    /// additional inaccessible.
    MsfsTheorem_8_2_ReflectiveTowerBoundedness,
    /// Historical claim labelled MSFS Theorem 5.1 (AFN-T α — Boundary Lemma): the absolute
    /// foundation stratum `𝓛_Abs` is empty.  The reflection tower
    /// CANNOT escape into `𝓛_Abs` because no Rich-metatheory
    /// admits a candidate satisfying (F_S) ∧ (Π_4) ∧ (Π_3-max)
    /// simultaneously.
    MsfsTheorem_5_1_AfntAlpha,
}

impl MsfsCitation {
    /// Stable diagnostic tag.
    pub fn tag(&self) -> &'static str {
        match self {
            MsfsCitation::BaseFootprint => "base_footprint",
            MsfsCitation::MsfsTheorem_9_6_MetaStabilisation => "msfs_theorem_9_6_meta_stabilisation",
            MsfsCitation::MsfsTheorem_8_2_ReflectiveTowerBoundedness => {
                "msfs_theorem_8_2_reflective_tower_boundedness"
            }
            MsfsCitation::MsfsTheorem_5_1_AfntAlpha => "msfs_theorem_5_1_afnt_alpha",
        }
    }

    /// Historical path relative to the MSFS corpus root; not a proof lookup.
    pub fn corpus_path(&self) -> &'static str {
        match self {
            MsfsCitation::BaseFootprint => "kernel/zfc_self_recognition.rs",
            MsfsCitation::MsfsTheorem_9_6_MetaStabilisation => {
                "theorems/msfs/09_meta_classification/theorems_9_3_9_4_9_6.vr"
            }
            MsfsCitation::MsfsTheorem_8_2_ReflectiveTowerBoundedness => {
                "theorems/msfs/08_bypass_paths/theorems_8_1_to_8_8.vr"
            }
            MsfsCitation::MsfsTheorem_5_1_AfntAlpha => {
                "theorems/msfs/05_afnt_alpha/theorem_5_1.vr"
            }
        }
    }

    /// Full descriptive sentence for human-readable audit output.
    pub fn description(&self) -> &'static str {
        match self {
            MsfsCitation::BaseFootprint => {
                "Per-rule footprint enumeration over ZFC + 2·κ \
                 (kernel_meta_soundness_footprint)"
            }
            MsfsCitation::MsfsTheorem_9_6_MetaStabilisation => {
                "Historical unchecked claim: MSFS Theorem 9.6 — Meta-classification stabilisation. \
                 𝔐^(Cls·k) ≃_2 𝔐^(Cls) for k ≥ 1 (theory-level idempotence; \
                 set-theoretic universe-ascent only)"
            }
            MsfsCitation::MsfsTheorem_8_2_ReflectiveTowerBoundedness => {
                "Historical unchecked claim: MSFS Theorem 8.2 — Reflective tower boundedness. \
                 Con(tower over S) = Con(S) + κ_inacc (exactly one extra \
                 strongly-inaccessible cardinal)"
            }
            MsfsCitation::MsfsTheorem_5_1_AfntAlpha => {
                "Historical unchecked claim: MSFS Theorem 5.1 (AFN-T α — Boundary Lemma) — 𝓛_Abs = ∅. \
                 The absolute foundation stratum is empty: no Rich-metatheory \
                 admits a candidate satisfying (F_S) ∧ (Π_4) ∧ (Π_3-max). \
                 The reflection tower cannot escape into 𝓛_Abs."
            }
        }
    }
}

// =============================================================================
// ReflectionStage — four legacy claim descriptors
// =============================================================================

/// Legacy stage descriptors. Their labels record mathematical claims, not
/// proofs that those claims hold or that these descriptors exhaust reflection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReflectionStage {
    /// `REF^0` — the base level. Per-rule footprint over ZFC + 2·κ.
    Base,
    /// Descriptor for the historical finite-stability claim.
    StableUnderUniverseAscent,
    /// Descriptor for the historical additional-universe bound claim.
    BoundedByOneInaccessible,
    /// Descriptor for the historical absolute-boundary claim.
    AbsoluteBoundaryEmpty,
}

impl ReflectionStage {
    /// Stable diagnostic tag.
    pub fn tag(&self) -> &'static str {
        match self {
            ReflectionStage::Base => "ref_0_base",
            ReflectionStage::StableUnderUniverseAscent => "ref_geq_1_stable",
            ReflectionStage::BoundedByOneInaccessible => "ref_omega_bounded",
            ReflectionStage::AbsoluteBoundaryEmpty => "ref_abs_empty",
        }
    }

    /// Short canonical name for audit output.
    pub fn name(&self) -> &'static str {
        match self {
            ReflectionStage::Base => "REF^0",
            ReflectionStage::StableUnderUniverseAscent => "REF^≥1",
            ReflectionStage::BoundedByOneInaccessible => "REF^ω",
            ReflectionStage::AbsoluteBoundaryEmpty => "REF^Abs",
        }
    }

    /// Historical reference attached to this stage descriptor.
    pub fn citation(&self) -> MsfsCitation {
        match self {
            ReflectionStage::Base => MsfsCitation::BaseFootprint,
            ReflectionStage::StableUnderUniverseAscent => {
                MsfsCitation::MsfsTheorem_9_6_MetaStabilisation
            }
            ReflectionStage::BoundedByOneInaccessible => {
                MsfsCitation::MsfsTheorem_8_2_ReflectiveTowerBoundedness
            }
            ReflectionStage::AbsoluteBoundaryEmpty => MsfsCitation::MsfsTheorem_5_1_AfntAlpha,
        }
    }

    /// The four legacy descriptors in their stable report order.
    pub fn full_list() -> [ReflectionStage; 4] {
        [
            ReflectionStage::Base,
            ReflectionStage::StableUnderUniverseAscent,
            ReflectionStage::BoundedByOneInaccessible,
            ReflectionStage::AbsoluteBoundaryEmpty,
        ]
    }
}

// =============================================================================
// Discharge predicates
// =============================================================================

/// Legacy numeric footprint predicate; the name does not grant proof authority.
pub fn base_discharges() -> bool {
    kernel_meta_soundness_holds()
}

/// Reuses the base footprint flag. No equivalence or stability theorem is proved.
pub fn stable_under_universe_ascent_discharges() -> bool {
    base_discharges()
}

/// Checks a declared finite budget against the legacy ceiling three.
/// This arithmetic comparison does not bound the consistency strength of a theory.
pub fn omega_bounded_discharges() -> bool {
    let max_inaccessible = max_inaccessible_required();
    // Legacy ceiling: 2 + 1 = 3; no consistency-strength theorem is checked.
    max_inaccessible <= 3
}

/// Legacy boundary-claim flag. The constant records an assertion; it neither
/// constructs a proof nor validates the cited mathematical theorem.
pub fn absolute_boundary_empty_discharges() -> bool {
    // Preserve the legacy assertion as observable data; proof_verdict does
    // not allow this flag to authorize a theorem.
    true
}

/// Greatest inaccessible-index required by any kernel rule.
fn max_inaccessible_required() -> u32 {
    use crate::zfc_self_recognition::InaccessibleLevel;
    let mut max_idx = 0u32;
    for rule in KernelRuleId::full_list() {
        for k in &required_meta_theory(rule).inaccessibles {
            let idx = match k {
                InaccessibleLevel::Kappa1 => 1,
                InaccessibleLevel::Kappa2 => 2,
            };
            if idx > max_idx {
                max_idx = idx;
            }
        }
    }
    max_idx
}

// =============================================================================
// Constructive MetaStabilisationWitness mirror
// =============================================================================
//
// The historical names below are retained for compatibility with diagnostic
// consumers. The underlying values are scalar descriptors, not proof objects.

/// Legacy boolean descriptor at one universe index. Despite the historical
/// type name, these flags are not categorical structures, maps or equivalences.
#[derive(Debug, Clone)]
pub struct ConstructiveMetaStabilisationWitness {
    /// The universe-index this witness instantiates. `k = 0` is
    /// the base; `k ≥ 1` is the iterated meta-classification level.
    pub universe_index: u32,
    /// (M1)–(M5) inheritance verdict — mirror of
    /// `a_m_cls_is_meta_cls`.
    pub a_m_cls_is_meta_cls_holds: bool,
    /// Theorem A.7 stabilisation verdict — mirror of
    /// `b_pi_inf_inf_plus_1_equivalent`.
    pub b_pi_inf_inf_plus_1_equivalent: bool,
    /// Theory-idempotence + universe-ascent verdict — mirror of
    /// `b_universe_ascent_with_theory_idempotence`.
    pub b_universe_ascent_with_theory_idempotence: bool,
}

impl ConstructiveMetaStabilisationWitness {
    /// Aggregate holds — every accessor must succeed for the
    /// witness to satisfy the Verum-side protocol's `requires`
    /// clause.
    pub fn holds(&self) -> bool {
        self.a_m_cls_is_meta_cls_holds
            && self.b_pi_inf_inf_plus_1_equivalent
            && self.b_universe_ascent_with_theory_idempotence
    }
}

/// Build the legacy descriptor: the declared footprint flag and two tests
/// for a positive index. The construction does not prove the properties named
/// by those flags.
pub fn synthesize_witness(k: u32) -> ConstructiveMetaStabilisationWitness {
    ConstructiveMetaStabilisationWitness {
        universe_index: k,
        a_m_cls_is_meta_cls_holds: base_discharges(),
        // Legacy descriptor convention: k >= 1 selects the positive-index branch.
        b_pi_inf_inf_plus_1_equivalent: k >= 1,
        b_universe_ascent_with_theory_idempotence: k >= 1,
    }
}

/// Observations from a numeric dispatch chain at one index. Historical type
/// names are retained for API compatibility; this value carries no proof term.
#[derive(Debug, Clone)]
pub struct ConstructiveDischarge {
    /// Universe-index this discharge applies to.
    pub universe_index: u32,
    /// Synthesised witness fed through the MSFS dispatch chain.
    pub witness: ConstructiveMetaStabilisationWitness,
    /// `kernel_truncate_to_level` verdict.  Theorem 9.6's proof
    /// body cites this intrinsic — surfacing the verdict here
    /// makes the dispatch chain inspectable.
    pub truncate_to_level_holds: bool,
    /// `kernel_straightening_equivalence` verdict.  Theorem 9.4
    /// (Meta-categoricity, the cousin theorem of 9.6) cites this
    /// intrinsic; the discharge chain leverages both.
    pub straightening_equivalence_holds: bool,
    /// Final verdict — the conjunction of the witness + dispatch
    /// outputs. This is what the audit gate reports.
    pub holds: bool,
}

/// Evaluate legacy scalar diagnostics at one index and preserve their outputs.
/// The conjunction is a diagnostic flag, not a discharge of reflection or
/// meta-classification obligations.
pub fn discharge_at_universe_index(k: u32) -> ConstructiveDischarge {
    let witness = synthesize_witness(k);

    // These scalar intrinsics report numeric observations, not equivalences.
    let truncate = dispatch_intrinsic(
        "kernel_truncate_to_level",
        &[IntrinsicValue::Int(k as i64), IntrinsicValue::Int(0)],
    );
    let straighten = dispatch_intrinsic(
        "kernel_straightening_equivalence",
        &[IntrinsicValue::Int(k as i64)],
    );

    let truncate_holds = matches!(
        truncate,
        Some(IntrinsicValue::Decision { holds: true, .. }),
    );
    let straighten_holds = matches!(
        straighten,
        Some(IntrinsicValue::Decision { holds: true, .. }),
    );

    // At the base (k = 0) the witness's b_* accessors are false by
    // construction, but the discharge holds because base_discharges
    // covers it. At k ≥ 1 the witness must hold and the dispatch
    // chain must agree.
    let holds = if k == 0 {
        base_discharges()
    } else {
        witness.holds() && truncate_holds && straighten_holds
    };

    ConstructiveDischarge {
        universe_index: k,
        witness,
        truncate_to_level_holds: truncate_holds,
        straightening_equivalence_holds: straighten_holds,
        holds,
    }
}

// =============================================================================
// Finite descriptor comparison
// =============================================================================

/// Comparison of the descriptor flags at finitely many indices. It reports
/// only the inspected interval, without a universally quantified conclusion.
#[derive(Debug, Clone)]
pub struct StabilityVerdict {
    /// Highest universe-ascent index `k` the walk reached.
    pub max_walked: u32,
    /// `true` iff the witness pattern was invariant across every walked level.
    pub all_stable: bool,
    /// First `k` where invariance broke; `None` when `all_stable` holds.
    pub divergence_at: Option<u32>,
    /// Sampled witness pattern from k=1 (the canonical reference);
    /// every subsequent k is compared against this.
    pub canonical_witness_summary: String,
}

impl StabilityVerdict {
    /// True iff the finite diagnostic walk completed without divergence.
    /// This does not establish stability at all universe levels.
    pub fn is_structurally_stable(&self) -> bool {
        self.all_stable && self.divergence_at.is_none()
    }

    /// The finite walk carries observations, not a universally quantified proof.
    pub fn proof_verdict(&self) -> crate::verdict::VerificationVerdict {
        use crate::verdict::{DischargeMethod, Evidence, VerificationVerdict};
        VerificationVerdict::StructuralOnly {
            method: DischargeMethod::MetaModeStability { max_lift: self.max_walked },
            evidence: Evidence::from_summary("Finite reflection diagnostic without theorem replay")
                .with("structurally_stable", self.is_structurally_stable().to_string()),
        }
    }

    /// Compatibility query for proof consumers; structural stability cannot discharge a theorem.
    pub fn is_load_bearing(&self) -> bool {
        self.proof_verdict().is_discharged()
    }
}

/// Compare descriptor flags for indices in `0..=max_lift` against index one.
/// This finite computation does not establish mathematical idempotence,
/// reflection, soundness, or stability beyond the sampled interval.
pub fn walk_stability_up_to(max_lift: u32) -> StabilityVerdict {
    let canonical = discharge_at_universe_index(1);
    let canonical_summary = format!(
        "a_m_cls={}, b_pi={}, b_universe_ascent={}, holds={}",
        canonical.witness.a_m_cls_is_meta_cls_holds,
        canonical.witness.b_pi_inf_inf_plus_1_equivalent,
        canonical.witness.b_universe_ascent_with_theory_idempotence,
        canonical.holds,
    );

    for k in 0..=max_lift {
        let d = discharge_at_universe_index(k);
        // For k = 0 the witness's b_* fields are false by
        // construction (the legacy positive-index convention).
        // For k ≥ 1 the witness pattern MUST equal the canonical
        // (the descriptor convention).
        if k >= 1 {
            let stable = d.witness.a_m_cls_is_meta_cls_holds
                == canonical.witness.a_m_cls_is_meta_cls_holds
                && d.witness.b_pi_inf_inf_plus_1_equivalent
                    == canonical.witness.b_pi_inf_inf_plus_1_equivalent
                && d.witness.b_universe_ascent_with_theory_idempotence
                    == canonical.witness.b_universe_ascent_with_theory_idempotence
                && d.holds == canonical.holds;
            if !stable {
                return StabilityVerdict {
                    max_walked: k,
                    all_stable: false,
                    divergence_at: Some(k),
                    canonical_witness_summary: canonical_summary,
                };
            }
        }
    }

    StabilityVerdict {
        max_walked: max_lift,
        all_stable: true,
        divergence_at: None,
        canonical_witness_summary: canonical_summary,
    }
}

// =============================================================================
// TowerReport — audit-gate carrier
// =============================================================================

/// One stage descriptor, its legacy reference, and a numeric diagnostic flag.
#[derive(Debug, Clone)]
pub struct StageVerdict {
    /// Human-readable stage name (`"Base"`, `"Stable"`, …).
    pub stage_name: &'static str,
    /// Stable diagnostic tag (`"ref_0"`, `"ref_geq_1"`, …).
    pub stage_tag: &'static str,
    /// MSFS theorem citation tag (`"theorem_9_6"`, …).
    pub citation_tag: &'static str,
    /// Path inside the MSFS corpus that carries the cited theorem.
    pub corpus_path: &'static str,
    /// True iff this stage's discharge function returned `true`.
    pub discharges: bool,
}

/// Structural observations and bibliography from the legacy reflection model.
/// Proof authority is reported separately and is never inferred from these flags.
#[derive(Debug, Clone)]
pub struct ReflectionTowerReport {
    /// Per-stage discharge verdicts in canonical order.
    pub stage_verdicts: Vec<StageVerdict>,
    /// Maximum inaccessible-index required by any kernel rule.
    /// Drives the `REF^ω` bound check via Theorem 8.2.
    pub max_inaccessible_required: u32,
    /// Scalar observations sampled at `k ∈ {0, 1, 2, 3, 7, 42}`.
    pub sampled_constructive_discharges: Vec<ConstructiveDischarge>,
    /// Finite descriptor comparison, with the first observed divergence.
    pub stability_verdict: StabilityVerdict,
}

impl ReflectionTowerReport {
    /// True iff the inspected flags are mutually consistent and positive.
    /// This is a property of the diagnostic data, not of the cited theories.
    pub fn is_structurally_consistent(&self) -> bool {
        self.stage_verdicts.iter().all(|v| v.discharges)
            && self
                .sampled_constructive_discharges
                .iter()
                .all(|d| d.holds)
            && self.stability_verdict.is_structurally_stable()
    }

    /// No report component binds the cited theorem to a checked proof term.
    pub fn proof_verdict(&self) -> crate::verdict::VerificationVerdict {
        use crate::verdict::{DischargeMethod, Evidence, VerificationVerdict};
        VerificationVerdict::StructuralOnly {
            method: DischargeMethod::MetaModeStability { max_lift: self.stability_verdict.max_walked },
            evidence: Evidence::from_summary("Reflection flags and references require statement-bound proof replay")
                .with("structurally_consistent", self.is_structurally_consistent().to_string()),
        }
    }

    /// True only for a proof discharge, never for numeric flags or citations.
    pub fn is_load_bearing(&self) -> bool {
        self.proof_verdict().is_discharged()
    }

    /// Number of stages with proof authority; this diagnostic API carries none.
    pub fn discharged_count(&self) -> usize {
        0
    }

    /// Number of sampled indices with proof authority; this API carries none.
    pub fn constructive_discharged_count(&self) -> usize {
        0
    }

    /// Number of stages whose diagnostic predicate returned true.
    pub fn structurally_positive_stage_count(&self) -> usize {
        self.stage_verdicts.iter().filter(|v| v.discharges).count()
    }

    /// Number of sampled per-index discharges that held.
    pub fn structurally_positive_sample_count(&self) -> usize {
        self.sampled_constructive_discharges
            .iter()
            .filter(|d| d.holds)
            .count()
    }
}

/// Evaluate the four legacy stage descriptors, sample selected indices and
/// record finite stability observations alongside their historical references.
pub fn build_tower_report() -> ReflectionTowerReport {
    let stage_verdicts: Vec<StageVerdict> = ReflectionStage::full_list()
        .iter()
        .map(|stage| {
            let citation = stage.citation();
            let discharges = match stage {
                ReflectionStage::Base => base_discharges(),
                ReflectionStage::StableUnderUniverseAscent => {
                    stable_under_universe_ascent_discharges()
                }
                ReflectionStage::BoundedByOneInaccessible => omega_bounded_discharges(),
                ReflectionStage::AbsoluteBoundaryEmpty => absolute_boundary_empty_discharges(),
            };
            StageVerdict {
                stage_name: stage.name(),
                stage_tag: stage.tag(),
                citation_tag: citation.tag(),
                corpus_path: citation.corpus_path(),
                discharges,
            }
        })
        .collect();
    // Finite, reproducible sample of the scalar descriptor.
    let sampled_constructive_discharges: Vec<ConstructiveDischarge> = [0, 1, 2, 3, 7, 42]
        .iter()
        .map(|&k| discharge_at_universe_index(k))
        .collect();
    // Compare only the finite interval [0, 10].
    let stability_verdict = walk_stability_up_to(10);
    ReflectionTowerReport {
        stage_verdicts,
        max_inaccessible_required: max_inaccessible_required(),
        sampled_constructive_discharges,
        stability_verdict,
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_stages_in_canonical_order() {
        let stages = ReflectionStage::full_list();
        assert_eq!(stages.len(), 4);
        assert_eq!(stages[0], ReflectionStage::Base);
        assert_eq!(stages[1], ReflectionStage::StableUnderUniverseAscent);
        assert_eq!(stages[2], ReflectionStage::BoundedByOneInaccessible);
        assert_eq!(stages[3], ReflectionStage::AbsoluteBoundaryEmpty);
    }

    #[test]
    fn stage_tags_distinct() {
        let tags: std::collections::BTreeSet<_> =
            ReflectionStage::full_list().iter().map(|s| s.tag()).collect();
        assert_eq!(tags.len(), 4);
    }

    #[test]
    fn citation_tags_distinct() {
        let cs = [
            MsfsCitation::BaseFootprint,
            MsfsCitation::MsfsTheorem_9_6_MetaStabilisation,
            MsfsCitation::MsfsTheorem_8_2_ReflectiveTowerBoundedness,
            MsfsCitation::MsfsTheorem_5_1_AfntAlpha,
        ];
        let tags: std::collections::BTreeSet<_> = cs.iter().map(|c| c.tag()).collect();
        assert_eq!(tags.len(), 4);
    }

    #[test]
    fn absolute_boundary_discharges_unconditionally() {
        // Pin: REF^Abs always discharges. The boundary's emptiness
        // is uniform across every Rich-metatheory + every
        // categorical level (MSFS Theorem 5.1 five-axis
        // absoluteness). The kernel never instantiates an 𝓛_Abs
        // candidate, so the discharge is unconditionally `true`.
        assert!(absolute_boundary_empty_discharges());
    }

    #[test]
    fn corpus_paths_resolve_when_corpus_is_available() {
        // Architectural pin (machine-checkable provenance): when
        // the MSFS corpus root is available via the environment
        // variable `VERUM_MSFS_CORPUS_DIR`, every citation's
        // corpus_path() must resolve on disk. This turns the
        // corpus_path() string from a documentation pointer into a
        // load-bearing build-time invariant: if the corpus moves
        // or is removed, the kernel knows immediately. When the
        // env var is unset (e.g., a downstream consumer building
        // verum_kernel without the corpus), the test is silently
        // skipped — the kernel does not require the corpus to
        // build.
        let corpus_root = match std::env::var("VERUM_MSFS_CORPUS_DIR") {
            Ok(s) => std::path::PathBuf::from(s),
            Err(_) => return,
        };
        for stage in ReflectionStage::full_list() {
            let path = stage.citation().corpus_path();
            if path.starts_with("kernel/") {
                continue;
            }
            let candidate = corpus_root.join(path);
            assert!(
                candidate.exists(),
                "MSFS corpus citation must resolve on disk; \
                 stage {} cited {}, looked at {}",
                stage.name(),
                path,
                candidate.display(),
            );
        }
    }

    #[test]
    fn citations_retain_historical_corpus_paths() {
        // Bibliographic identifiers stay inspectable without proof authority.
        for c in [
            MsfsCitation::BaseFootprint,
            MsfsCitation::MsfsTheorem_9_6_MetaStabilisation,
            MsfsCitation::MsfsTheorem_8_2_ReflectiveTowerBoundedness,
        ] {
            assert!(!c.corpus_path().is_empty());
        }
        // Theorem 9.6 lives under 09_meta_classification.
        assert!(
            MsfsCitation::MsfsTheorem_9_6_MetaStabilisation
                .corpus_path()
                .contains("09_meta_classification"),
        );
        // Theorem 8.2 lives under 08_bypass_paths.
        assert!(
            MsfsCitation::MsfsTheorem_8_2_ReflectiveTowerBoundedness
                .corpus_path()
                .contains("08_bypass_paths"),
        );
    }

    #[test]
    fn base_discharges_under_current_kernel() {
        assert!(base_discharges(), "REF^0 must discharge for the current kernel rule roster");
    }

    #[test]
    fn stable_under_universe_ascent_reduces_to_base() {
        // Soundness pin: per MSFS Theorem 9.6(b), REF^≥1 reduces to
        // base_discharges. The reduction is the entire architectural
        // gain of grounding in MSFS — without 9.6 we would have
        // separate discharge logic per universe-index k.
        assert_eq!(
            stable_under_universe_ascent_discharges(),
            base_discharges(),
        );
    }

    #[test]
    fn omega_bounded_holds_when_max_inaccessible_at_most_three() {
        // Theorem 8.2 bounds the tower by Con(S) + κ_inacc (one
        // extra inaccessible). With the standard 2·κ base, the
        // bound is 3 inaccessibles. The current kernel uses κ_1
        // and κ_2, max_idx = 2 ≤ 3.
        assert!(omega_bounded_discharges());
        assert!(max_inaccessible_required() <= 3);
    }

    #[test]
    fn build_tower_report_is_structurally_consistent() {
        let r = build_tower_report();
        assert_eq!(r.stage_verdicts.len(), 4);
        assert!(r.is_structurally_consistent());
        assert_eq!(r.structurally_positive_stage_count(), 4);
    }

    #[test]
    fn tower_report_field_consistency() {
        let report = build_tower_report();
        for (verdict, stage) in report
            .stage_verdicts
            .iter()
            .zip(ReflectionStage::full_list().iter())
        {
            assert_eq!(verdict.stage_name, stage.name());
            assert_eq!(verdict.stage_tag, stage.tag());
            assert_eq!(verdict.citation_tag, stage.citation().tag());
            assert_eq!(verdict.corpus_path, stage.citation().corpus_path());
        }
    }

    // ----- Constructive per-index discharge -----

    #[test]
    fn synthesize_witness_k_zero_is_base() {
        let w = synthesize_witness(0);
        assert_eq!(w.universe_index, 0);
        // At k=0, b_* accessors are false by construction
        // (stabilisation kicks in at k≥1).
        assert!(!w.b_pi_inf_inf_plus_1_equivalent);
        assert!(!w.b_universe_ascent_with_theory_idempotence);
        // a_m_cls accessor mirrors base footprint.
        assert_eq!(w.a_m_cls_is_meta_cls_holds, base_discharges());
    }

    #[test]
    fn synthesize_witness_k_one_holds_under_current_kernel() {
        let w = synthesize_witness(1);
        assert!(w.b_pi_inf_inf_plus_1_equivalent);
        assert!(w.b_universe_ascent_with_theory_idempotence);
        assert!(w.holds());
    }

    #[test]
    fn synthesize_witness_idempotence_at_arbitrary_k() {
        // Compare the actual scalar flags at the selected indices.
        let w1 = synthesize_witness(1);
        let w2 = synthesize_witness(2);
        let w7 = synthesize_witness(7);
        let w999 = synthesize_witness(999);
        for w in [&w2, &w7, &w999] {
            assert_eq!(w.a_m_cls_is_meta_cls_holds, w1.a_m_cls_is_meta_cls_holds);
            assert_eq!(
                w.b_pi_inf_inf_plus_1_equivalent,
                w1.b_pi_inf_inf_plus_1_equivalent,
            );
            assert_eq!(
                w.b_universe_ascent_with_theory_idempotence,
                w1.b_universe_ascent_with_theory_idempotence,
            );
        }
    }

    #[test]
    fn discharge_at_universe_index_zero_falls_back_to_base() {
        let d = discharge_at_universe_index(0);
        assert_eq!(d.universe_index, 0);
        assert_eq!(d.holds, base_discharges());
    }

    #[test]
    fn discharge_at_universe_index_one_has_positive_diagnostics() {
        // Positive scalar predicates must remain inspectable.
        let d = discharge_at_universe_index(1);
        assert!(d.holds, "REF^1 diagnostic predicate must hold under the legacy model");
        assert!(d.witness.holds());
        assert!(d.truncate_to_level_holds);
        assert!(d.straightening_equivalence_holds);
    }

    #[test]
    fn discharge_at_arbitrary_universe_indices_are_idempotent() {
        // The selected numeric observations coincide.
        let verdicts: Vec<bool> = [1u32, 2, 3, 7, 42, 999]
            .iter()
            .map(|&k| discharge_at_universe_index(k).holds)
            .collect();
        let first = verdicts[0];
        for v in verdicts.iter().skip(1) {
            assert_eq!(
                *v, first,
                "selected numeric descriptors must agree",
            );
        }
    }

    #[test]
    fn report_includes_sampled_constructive_discharges() {
        let r = build_tower_report();
        assert_eq!(
            r.sampled_constructive_discharges.len(),
            6,
            "report must sample 6 universe-indices: 0, 1, 2, 3, 7, 42",
        );
        // Every sampled discharge must hold.
        assert!(r.is_structurally_consistent());
        assert_eq!(r.structurally_positive_sample_count(), 6);
    }

    // ----- Multi-level stability walk -----

    #[test]
    fn walk_stability_holds_for_max_lift_zero() {
        // Pin: max_lift=0 walks only the base level — there are no
        // k≥1 to compare, so the verdict is trivially stable.
        let v = walk_stability_up_to(0);
        assert!(v.all_stable);
        assert!(v.divergence_at.is_none());
        assert_eq!(v.max_walked, 0);
    }

    #[test]
    fn walk_stability_holds_for_default_max_lift_10() {
        // The selected finite sample must be reported accurately.
        let v = walk_stability_up_to(10);
        assert!(v.is_structurally_stable(), "stability walk must hold; got {:?}", v);
        assert_eq!(v.max_walked, 10);
        assert!(v.divergence_at.is_none());
    }

    #[test]
    fn walk_stability_at_arbitrary_high_lift() {
        // Extend the finite diagnostic sample; no universal claim follows.
        let v = walk_stability_up_to(100);
        assert!(v.is_structurally_stable());
    }

    #[test]
    fn build_tower_report_includes_stability_verdict() {
        let r = build_tower_report();
        assert!(r.stability_verdict.is_structurally_stable());
        assert_eq!(r.stability_verdict.max_walked, 10);
    }

    #[test]
    fn architectural_pin_msfs_grounding_not_generic_proof_theory() {
        // Preserve provenance labels, independently of proof status.
        for stage in ReflectionStage::full_list() {
            let c = stage.citation();
            let path = c.corpus_path();
            // Every path either lives in the MSFS corpus
            // theorems/msfs/ tree or points at the kernel
            // base-footprint (zfc_self_recognition).
            assert!(
                path.contains("theorems/msfs/") || path.contains("zfc_self_recognition"),
                "stage {} citation must point at MSFS corpus or kernel infrastructure, got: {}",
                stage.name(),
                path,
            );
        }
    }
}
