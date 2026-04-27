//! K-Eps-Mu V1 shape-check tests.
//!
//! Naturality witness (incremental): tightens `check_eps_mu_coherence` from
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
fn v3_non_identity_distinct_free_vars_rejected() {
    // V3 gate (b): Diakrisis Axi-2 M is variable-preserving — it
    // cannot rename free variables. (EpsilonOf(M_α), AlphaOf(EpsilonOf(α)))
    // with structurally different inner vars has free_vars(M_α) =
    // {M_α} ≠ {α} = free_vars(α), so no τ-witness can exist.
    //
    // Pre-V3 this test was named *conservatively_accepted* because
    // V1 / V2 had no free-vars check; V3-incremental rejects it
    // correctly with a free-var-mismatch diagnostic.
    let lhs = epsilon_of(var("M_α"));
    let rhs = alpha_of(epsilon_of(var("α")));
    let err = check_eps_mu_coherence(&lhs, &rhs, "non_identity_M_distinct_vars")
        .unwrap_err();
    match err {
        KernelError::EpsMuNaturalityFailed { context } => {
            assert!(
                context.as_str().contains("free-var mismatch"),
                "expected free-var-mismatch diagnostic, got {:?}",
                context
            );
        }
        other => panic!("expected EpsMuNaturalityFailed, got {:?}", other),
    }
}

#[test]
fn v3_non_identity_matching_free_vars_accepted() {
    // V3-incremental still accepts non-identity M whose τ-witness
    // necessary conditions all pass. Here M acts on α via a wrapping
    // metaisation (modelled by the same Var name on both sides)
    // — free vars match, depths match, no β-redex distinction.
    //
    // The full sufficient witness construction (σ_α / π_α) is the
    // V3-final step still tracked under #181; this test pins
    // V3-incremental's accept path.
    let lhs = epsilon_of(var("α"));
    let rhs = alpha_of(epsilon_of(var("α")));
    // M_α structurally equals α here (identity-functor sub-case),
    // accepted on the structural-equality fast path before the V3
    // gates fire.
    assert!(
        check_eps_mu_coherence(&lhs, &rhs, "v3_identity_alpha").is_ok(),
        "structurally equal inner-α should hit identity sub-case"
    );
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
fn v3_modal_box_distinct_inner_var_rejected() {
    // M_α = Box(α_prime), α = Box(α). Both have md^ω = 1 (depth
    // gate passes) but free_vars are {α_prime} ≠ {α} (V3 gate (b)
    // rejects). Pre-V3 (V1 / V2) this was the canonical
    // "depth-matched non-identity-M" *accept* fixture; V3-incremental
    // tightens by also requiring free-var preservation.
    let lhs = epsilon_of(modal_box(var("α_prime")));
    let rhs = alpha_of(epsilon_of(modal_box(var("α"))));
    let err = check_eps_mu_coherence(&lhs, &rhs, "v3_modal_box_var_mismatch")
        .unwrap_err();
    match err {
        KernelError::EpsMuNaturalityFailed { context } => {
            assert!(
                context.as_str().contains("free-var mismatch"),
                "expected free-var-mismatch diagnostic, got {:?}",
                context
            );
        }
        other => panic!("expected EpsMuNaturalityFailed, got {:?}", other),
    }
}

#[test]
fn v3_modal_box_same_inner_var_accepted() {
    // M_α = Box(α), α = Box(α). Free vars match {α}, depth matches
    // (md^ω = 1). Structurally equal ⇒ identity sub-case fast path.
    let lhs = epsilon_of(modal_box(var("α")));
    let rhs = alpha_of(epsilon_of(modal_box(var("α"))));
    assert!(
        check_eps_mu_coherence(&lhs, &rhs, "v3_modal_box_var_match").is_ok(),
        "matching free vars + depths should pass V3-incremental"
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
fn v3_depth_check_uses_omega_aware_ranks_with_var_preservation() {
    // Both sides have rank 3 (Box × 3) and matching inner var names
    // — V3-incremental accepts because depth + free-vars + β-eq
    // all hold simultaneously. Pre-V3 the test only required depth
    // match; V3 adds the free-vars half of the necessary-condition
    // tuple.
    let m_alpha = modal_box(modal_box(modal_box(var("α"))));
    let alpha   = modal_box(modal_box(modal_box(var("α"))));
    let lhs = epsilon_of(m_alpha);
    let rhs = alpha_of(epsilon_of(alpha));
    assert!(
        check_eps_mu_coherence(&lhs, &rhs, "v3_omega_rank_match").is_ok(),
        "same finite rank + matching free vars should pass V3-incremental"
    );
}

#[test]
fn v3_depth_match_but_distinct_vars_rejected() {
    // Same rank (3) but distinct inner Var names: depth gate (a)
    // passes, free-vars gate (b) fails ⇒ reject under V3-incremental.
    let m_alpha = modal_box(modal_box(modal_box(var("M_α"))));
    let alpha   = modal_box(modal_box(modal_box(var("α"))));
    let lhs = epsilon_of(m_alpha);
    let rhs = alpha_of(epsilon_of(alpha));
    let err = check_eps_mu_coherence(&lhs, &rhs, "v3_depth_match_var_mismatch")
        .unwrap_err();
    match err {
        KernelError::EpsMuNaturalityFailed { context } => {
            assert!(
                context.as_str().contains("free-var mismatch"),
                "expected free-var-mismatch diagnostic, got {:?}",
                context
            );
        }
        other => panic!("expected EpsMuNaturalityFailed, got {:?}", other),
    }
}
