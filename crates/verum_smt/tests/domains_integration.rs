//! Integration tests for Phase D.3 SMT domain encodings.

use verum_smt::domains::{
    epistemic::{
        epistemic_axioms, verify_invariants_preserved, EpistemicInvariant, EpistemicResult,
        PartialTrace, ProjectiveMeasurement,
    },
    sheaf::{verify_descent, DescentProblem, DescentResult},
};

// ==================== Sheaf descent ====================

#[test]
fn sheaf_empty_cover_trivial_descent() {
    let p = DescentProblem::new("c");
    assert_eq!(verify_descent(&p), DescentResult::EmptyCover);
}

#[test]
fn sheaf_single_cover_with_compatibility() {
    let p = DescentProblem::new("c")
        .add_cover("f", "s")
        .with_compatibility();
    assert_eq!(verify_descent(&p), DescentResult::UniqueGlobalSection);
}

#[test]
fn sheaf_five_way_cover_descends() {
    let mut p = DescentProblem::new("X").with_compatibility();
    for i in 0..5 {
        p = p.add_cover(format!("f{}", i), format!("s{}", i));
    }
    assert_eq!(verify_descent(&p), DescentResult::UniqueGlobalSection);
}

#[test]
fn sheaf_missing_compatibility_blocks_descent() {
    let p = DescentProblem::new("c")
        .add_cover("f1", "s1")
        .add_cover("f2", "s2");
    assert_eq!(
        verify_descent(&p),
        DescentResult::CompatibilityNotVerified
    );
}

#[test]
fn sheaf_mismatched_sections_undetermined() {
    let mut p = DescentProblem::new("c").with_compatibility();
    p.cover.push("f1".into());
    p.cover.push("f2".into());
    p.cover.push("f3".into());
    p.local_sections.push("s1".into());
    // 3 covers, 1 section — mismatch
    assert_eq!(verify_descent(&p), DescentResult::Undetermined);
}

// ==================== Epistemic states ====================

#[test]
fn epistemic_valid_pure_state() {
    let inv = EpistemicInvariant::new(2)
        .with_psd(true)
        .with_normalized_trace(true);
    assert!(inv.is_valid());
}

#[test]
fn epistemic_zero_dim_invalid() {
    let inv = EpistemicInvariant::new(0)
        .with_psd(true)
        .with_normalized_trace(true);
    assert!(!inv.is_valid());
}

#[test]
fn epistemic_psd_violation_detected() {
    let pre = EpistemicInvariant::new(2)
        .with_psd(true)
        .with_normalized_trace(true);
    let post = EpistemicInvariant::new(2)
        .with_psd(false)
        .with_normalized_trace(true);
    assert_eq!(
        verify_invariants_preserved(&pre, &post),
        EpistemicResult::PsdViolated
    );
}

#[test]
fn epistemic_trace_violation_detected() {
    let pre = EpistemicInvariant::new(2)
        .with_psd(true)
        .with_normalized_trace(true);
    let post = EpistemicInvariant::new(2)
        .with_psd(true)
        .with_normalized_trace(false);
    assert_eq!(
        verify_invariants_preserved(&pre, &post),
        EpistemicResult::TraceViolated
    );
}

#[test]
fn epistemic_all_invariants_preserved() {
    let pre = EpistemicInvariant::new(4)
        .with_psd(true)
        .with_normalized_trace(true);
    let post = EpistemicInvariant::new(4)
        .with_psd(true)
        .with_normalized_trace(true);
    assert_eq!(
        verify_invariants_preserved(&pre, &post),
        EpistemicResult::InvariantsPreserved
    );
}

// ==================== Projective measurement ====================

#[test]
fn projective_measurement_well_formed() {
    let pre = EpistemicInvariant::new(4)
        .with_psd(true)
        .with_normalized_trace(true);
    let m = ProjectiveMeasurement::new(pre, 4);
    assert!(m.is_well_formed());
}

#[test]
fn projective_measurement_dim_mismatch_fails() {
    let pre = EpistemicInvariant::new(4)
        .with_psd(true)
        .with_normalized_trace(true);
    let m = ProjectiveMeasurement::new(pre, 2);
    assert!(!m.is_well_formed());
}

// ==================== Partial trace (CPTP map) ====================

#[test]
fn partial_trace_reduces_dim() {
    let pt = PartialTrace::new(8, 4);
    assert!(pt.is_valid_cptp_map());
}

#[test]
fn partial_trace_cannot_increase_dim() {
    let pt = PartialTrace::new(2, 8);
    assert!(!pt.is_valid_cptp_map());
}

#[test]
fn partial_trace_preserves_psd_and_trace() {
    let pt = PartialTrace::new(4, 2);
    assert!(pt.preserves_psd);
    assert!(pt.preserves_trace);
    assert!(pt.is_valid_cptp_map());
}

// ==================== Axiom seeding ====================

#[test]
fn axiom_seeding_non_empty() {
    let axioms = epistemic_axioms();
    assert_eq!(axioms.len(), 5);
    // Verify each axiom has meaningful content
    for axiom in &axioms {
        assert!(!axiom.as_str().is_empty());
        assert!(axiom.as_str().contains("ρ") || axiom.as_str().contains("ρ"));
    }
}
