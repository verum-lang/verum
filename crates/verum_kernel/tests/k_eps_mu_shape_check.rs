//! K-Eps-Mu V1 shape-check tests.
//!
//! VFE-1 V1 (incremental): tightens `check_eps_mu_coherence` from
//! V0's permissive `(EpsilonOf(_), AlphaOf(_))` accept-anything
//! skeleton to a structural shape check that:
//!
//!   • accepts the identity-naturality case (`lhs == rhs` structurally),
//!   • accepts the canonical naturality-square shape
//!     `(EpsilonOf(M_α), AlphaOf(EpsilonOf(α)))`,
//!   • rejects malformed pairs like `(EpsilonOf(_), AlphaOf(t))`
//!     where `t` is not itself an `EpsilonOf` constructor.
//!
//! The full τ-witness construction (σ_α from Code_S morphism + π_α
//! from Perform_{ε_math} naturality through axiom A-3) is V2 work
//! tracked under the multi-week K-Eps-Mu naturality witness item.

use verum_common::{Heap, Text};
use verum_kernel::{check_eps_mu_coherence, CoreTerm, KernelError};

fn var(name: &str) -> CoreTerm {
    CoreTerm::Var(Text::from(name))
}

fn epsilon_of(t: CoreTerm) -> CoreTerm {
    CoreTerm::EpsilonOf(Heap::new(t))
}

fn alpha_of(t: CoreTerm) -> CoreTerm {
    CoreTerm::AlphaOf(Heap::new(t))
}

#[test]
fn identity_naturality_square_accepted_structurally() {
    let alpha = epsilon_of(var("α"));
    let lhs = alpha.clone();
    let rhs = alpha;
    assert!(check_eps_mu_coherence(&lhs, &rhs, "identity_square").is_ok());
}

#[test]
fn identity_naturality_square_accepted_for_bare_terms() {
    let lhs = var("α");
    let rhs = var("α");
    assert!(check_eps_mu_coherence(&lhs, &rhs, "identity_bare").is_ok());
}

#[test]
fn canonical_naturality_square_accepted_for_identity_functor() {
    // (EpsilonOf(α), AlphaOf(EpsilonOf(α))) — the M = id case.
    let alpha = var("α");
    let lhs = epsilon_of(alpha.clone());
    let rhs = alpha_of(epsilon_of(alpha));
    assert!(check_eps_mu_coherence(&lhs, &rhs, "M_eq_id_case").is_ok());
}

#[test]
fn naturality_with_non_identity_functor_conservatively_accepted() {
    // V1 is conservative for non-identity M (per the function docs):
    // (EpsilonOf(M_α), AlphaOf(EpsilonOf(α))) where M_α ≠ α
    // structurally — the τ-witness is V2 work; V1 accepts.
    let lhs = epsilon_of(var("M_α"));
    let rhs = alpha_of(epsilon_of(var("α")));
    assert!(check_eps_mu_coherence(&lhs, &rhs, "non_identity_M").is_ok());
}

#[test]
fn malformed_alpha_of_inner_non_epsilon_rejected() {
    // (EpsilonOf(_), AlphaOf(Var(_))) — inner of AlphaOf is not an
    // EpsilonOf, which violates the canonical naturality-square
    // shape. V1 must reject this; V0 incorrectly accepted it.
    let lhs = epsilon_of(var("M_α"));
    let rhs = alpha_of(var("α"));
    let err = check_eps_mu_coherence(&lhs, &rhs, "malformed_inner")
        .unwrap_err();
    match err {
        KernelError::EpsMuNaturalityFailed { context } => {
            assert_eq!(context.as_str(), "malformed_inner");
        }
        other => panic!("expected EpsMuNaturalityFailed, got {:?}", other),
    }
}

#[test]
fn unrelated_term_pair_rejected() {
    let lhs = var("α");
    let rhs = var("β");
    let err = check_eps_mu_coherence(&lhs, &rhs, "unrelated_pair")
        .unwrap_err();
    assert!(matches!(err, KernelError::EpsMuNaturalityFailed { .. }));
}

#[test]
fn alpha_of_without_epsilon_of_lhs_rejected() {
    // (Var(_), AlphaOf(EpsilonOf(_))) — lhs is not an EpsilonOf,
    // so the canonical shape doesn't match.
    let lhs = var("α");
    let rhs = alpha_of(epsilon_of(var("α")));
    let err = check_eps_mu_coherence(&lhs, &rhs, "missing_lhs_epsilon")
        .unwrap_err();
    assert!(matches!(err, KernelError::EpsMuNaturalityFailed { .. }));
}

#[test]
fn deeply_nested_identity_accepted_via_partial_eq() {
    let inner = alpha_of(epsilon_of(var("α")));
    let lhs = epsilon_of(inner.clone());
    let rhs = epsilon_of(inner);
    assert!(check_eps_mu_coherence(&lhs, &rhs, "nested_identity").is_ok());
}

// ============================================================================
// V2 increment: modal-depth preservation pre-condition (#189)
//
// The natural-equivalence τ : ε ∘ M ≃ A ∘ ε is depth-preserving;
// for non-identity M, m_depth_omega(M_α) must equal m_depth_omega(α)
// or no τ-witness can possibly exist. V2 rejects such depth-
// mismatched pairs (V1 conservatively accepted them).
// ============================================================================

fn modal_box(t: CoreTerm) -> CoreTerm {
    CoreTerm::ModalBox(Heap::new(t))
}

#[test]
fn v2_non_identity_with_matching_depths_still_accepted() {
    // M_α = Box(α') with rank 1; α = Box(α) with rank 1.
    // Both have md^ω = 1 ⇒ depth precondition holds ⇒ V2 accepts
    // (V3 / #181 will sharpen with the actual τ-witness check).
    let lhs = epsilon_of(modal_box(var("α_prime")));
    let rhs = alpha_of(epsilon_of(modal_box(var("α"))));
    assert!(
        check_eps_mu_coherence(&lhs, &rhs, "v2_matching_depth").is_ok(),
        "depth-matched non-identity M must still pass (V2 conservative-accept)"
    );
}

#[test]
fn v2_non_identity_with_mismatched_depths_rejected() {
    // M_α = Box(Box(α)) with rank 2; α = α with rank 0.
    // Depth mismatch ⇒ no τ-witness possible ⇒ V2 rejects.
    // V1 would have conservatively accepted.
    let lhs = epsilon_of(modal_box(modal_box(var("α"))));
    let rhs = alpha_of(epsilon_of(var("α")));
    let err = check_eps_mu_coherence(&lhs, &rhs, "v2_depth_mismatch")
        .expect_err("V2 must reject depth-mismatched non-identity pair");
    match err {
        KernelError::EpsMuNaturalityFailed { context } => {
            // V2.5: context now embeds the depth-mismatch values
            // for post-mortem readability. Substring-match so the
            // assertion stays robust if the message wording shifts.
            let s = context.as_str();
            assert!(
                s.contains("v2_depth_mismatch"),
                "context should preserve caller tag: {}",
                s,
            );
            assert!(
                s.contains("md^ω(M_α)=2"),
                "context should surface lhs depth: {}",
                s,
            );
            assert!(
                s.contains("md^ω(α)=0"),
                "context should surface rhs depth: {}",
                s,
            );
        }
        other => panic!("expected EpsMuNaturalityFailed, got {:?}", other),
    }
}

#[test]
fn v2_identity_case_unaffected_by_depth_check() {
    // M = id ⇒ M_α == α ⇒ identity-functor arm fires before
    // the depth check. Even if depths happened to be unusual,
    // structural equality short-circuits.
    let alpha = modal_box(modal_box(var("α")));
    let lhs = epsilon_of(alpha.clone());
    let rhs = alpha_of(epsilon_of(alpha));
    assert!(check_eps_mu_coherence(&lhs, &rhs, "v2_identity").is_ok());
}

#[test]
fn v2_depth_check_uses_omega_aware_ranks() {
    // Both sides reach into ω-rank territory (Box(Box(...)) ranks
    // are finite, but the depth comparison is total over the
    // OrdinalDepth lattice — pure-finite k1 == pure-finite k2
    // iff k1 == k2.
    let m_alpha = modal_box(modal_box(modal_box(var("M_α"))));
    let alpha   = modal_box(modal_box(modal_box(var("α"))));
    // Same rank (3), different inner Var names ⇒ structurally
    // unequal but depth-matched ⇒ V2 accepts.
    let lhs = epsilon_of(m_alpha);
    let rhs = alpha_of(epsilon_of(alpha));
    assert!(
        check_eps_mu_coherence(&lhs, &rhs, "v2_omega_rank_match").is_ok(),
        "same finite rank should pass V2"
    );
}
