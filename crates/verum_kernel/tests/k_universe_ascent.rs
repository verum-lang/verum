//! K-Universe-Ascent integration tests (Categorical coherence, Theorem 131.T).
//!
//! The κ-tower kernel rule from `core.math.frameworks.diakrisis_stack_model`
//! lifted to the kernel layer. Per Theorem 134.T (tight 2-inacc bound),
//! only two non-trivial universe levels exist; valid transitions are:
//!
//!   • Truncated → Truncated   (Cat-baseline identity)
//!   • κ_1 → κ_1                (identity at κ_1)
//!   • κ_1 → κ_2                (Lemma 131.L1 ascent)
//!   • κ_2 → κ_2                (Lemma 131.L3 Drake-reflection closure)
//!
//! All other transitions — Truncated → κ_*, κ_2 → κ_1, etc. — are
//! rejected with `KernelError::UniverseAscentInvalid`.

use verum_kernel::{KernelError, UniverseTier, check_universe_ascent};

#[test]
fn truncated_identity_accepted() {
    assert!(check_universe_ascent(
        UniverseTier::Truncated,
        UniverseTier::Truncated,
        "cat_baseline_identity"
    )
    .is_ok());
}

#[test]
fn kappa_1_identity_accepted() {
    assert!(check_universe_ascent(
        UniverseTier::Kappa1,
        UniverseTier::Kappa1,
        "kappa_1_identity"
    )
    .is_ok());
}

#[test]
fn kappa_1_to_kappa_2_accepted() {
    // The canonical Lemma 131.L1 ascent.
    assert!(check_universe_ascent(
        UniverseTier::Kappa1,
        UniverseTier::Kappa2,
        "M_stack_ascent"
    )
    .is_ok());
}

#[test]
fn kappa_2_drake_closure_accepted() {
    // Lemma 131.L3: M_stack on a κ_2 articulation stays at κ_2.
    assert!(check_universe_ascent(
        UniverseTier::Kappa2,
        UniverseTier::Kappa2,
        "drake_reflection_closure"
    )
    .is_ok());
}

#[test]
fn truncated_to_kappa_1_rejected() {
    // Truncated must not be the source for ascent — the user
    // should have lifted to κ_1 first.
    let err = check_universe_ascent(
        UniverseTier::Truncated,
        UniverseTier::Kappa1,
        "invalid_truncated_to_kappa_1",
    )
    .unwrap_err();
    match err {
        KernelError::UniverseAscentInvalid { .. } => {}
        other => panic!("expected UniverseAscentInvalid, got {:?}", other),
    }
}

#[test]
fn truncated_to_kappa_2_rejected() {
    let err = check_universe_ascent(
        UniverseTier::Truncated,
        UniverseTier::Kappa2,
        "invalid_truncated_to_kappa_2",
    )
    .unwrap_err();
    match err {
        KernelError::UniverseAscentInvalid { .. } => {}
        other => panic!("expected UniverseAscentInvalid, got {:?}", other),
    }
}

#[test]
fn kappa_2_to_kappa_1_rejected() {
    // Tier inversion — κ_2 articulations cannot descend to κ_1
    // through M_stack.
    let err = check_universe_ascent(
        UniverseTier::Kappa2,
        UniverseTier::Kappa1,
        "invalid_kappa_2_to_kappa_1",
    )
    .unwrap_err();
    match err {
        KernelError::UniverseAscentInvalid { .. } => {}
        other => panic!("expected UniverseAscentInvalid, got {:?}", other),
    }
}

#[test]
fn kappa_2_to_truncated_rejected() {
    // Cannot collapse κ_2 back to the Cat-baseline.
    let err = check_universe_ascent(
        UniverseTier::Kappa2,
        UniverseTier::Truncated,
        "invalid_kappa_2_to_truncated",
    )
    .unwrap_err();
    match err {
        KernelError::UniverseAscentInvalid { .. } => {}
        other => panic!("expected UniverseAscentInvalid, got {:?}", other),
    }
}

#[test]
fn kappa_1_to_truncated_rejected() {
    let err = check_universe_ascent(
        UniverseTier::Kappa1,
        UniverseTier::Truncated,
        "invalid_kappa_1_to_truncated",
    )
    .unwrap_err();
    match err {
        KernelError::UniverseAscentInvalid { .. } => {}
        other => panic!("expected UniverseAscentInvalid, got {:?}", other),
    }
}

#[test]
fn universe_tier_strict_ordering_holds() {
    // Truncated < κ_1 < κ_2 strict ordering.
    assert!(UniverseTier::Truncated.lt(&UniverseTier::Kappa1));
    assert!(UniverseTier::Truncated.lt(&UniverseTier::Kappa2));
    assert!(UniverseTier::Kappa1.lt(&UniverseTier::Kappa2));

    // No reflexive lt.
    assert!(!UniverseTier::Truncated.lt(&UniverseTier::Truncated));
    assert!(!UniverseTier::Kappa1.lt(&UniverseTier::Kappa1));
    assert!(!UniverseTier::Kappa2.lt(&UniverseTier::Kappa2));

    // No reverse.
    assert!(!UniverseTier::Kappa2.lt(&UniverseTier::Kappa1));
    assert!(!UniverseTier::Kappa1.lt(&UniverseTier::Truncated));
}

#[test]
fn universe_tier_succ_saturates_at_kappa_2() {
    // Per Lemma 131.L3 / Theorem 134.T: succ(κ_2) = κ_2; no κ_3.
    assert_eq!(UniverseTier::Truncated.succ(), UniverseTier::Kappa1);
    assert_eq!(UniverseTier::Kappa1.succ(), UniverseTier::Kappa2);
    assert_eq!(UniverseTier::Kappa2.succ(), UniverseTier::Kappa2);
}
