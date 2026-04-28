//! K-Eps-Mu kernel rule — split per #198.
//!
//! Verifies the canonical 2-natural equivalence
//!
//! ```text
//!     τ : ε ∘ M ≃ A ∘ ε
//! ```
//!
//! from Diakrisis Proposition 5.1 / Corollary 5.10 (ν = e ∘ ε). The
//! kernel uses this rule as a structural witness that articulation
//! depth and enactment depth are connected by the canonical
//! biadjunction's transferred unit/counit.
//!
//! V0/V1/V2 staging is documented in
//! `docs/architecture/foundational-extensions.md` §1.7.

use verum_common::Text;

use crate::depth::m_depth_omega;
use crate::diakrisis_bridge::{BridgeAudit, admit_eps_mu_tau_witness};
use crate::support::{definitional_eq, free_vars};
use crate::{CoreTerm, KernelError};

/// Naturality witness V0 → V3-incremental — `K-Eps-Mu` kernel rule.
///
/// V0 shipped a permissive skeleton accepting any
/// `(EpsilonOf(_), AlphaOf(_))`. V1 tightened the shape check. V2
/// added modal-depth preservation. V3-incremental (this revision)
/// adds two further necessary conditions, narrowing the gap to the
/// full σ_α/π_α witness construction:
///
///   • `(EpsilonOf(M_α), AlphaOf(EpsilonOf(α)))` is the canonical
///     naturality-square shape per Proposition 5.1 / Corollary 5.10.
///
///   • `(t, t)` (structurally equal) is the degenerate identity
///     square — always accepted.
///
///   • Identity-functor sub-case (`m_alpha == α_rhs` structurally)
///     accepts directly: identity naturality commutes trivially.
///
///   • For the non-identity-M case, V3 fires three gates, all
///     **necessary** for the τ-witness to exist; if any fails we
///     know no witness can exist and reject. Gate (a) — modal-depth
///     preservation (V2-shipped): the natural equivalence τ is an
///     (∞,1)-equivalence, hence depth-preserving;
///     `m_depth_omega(M_α) ≠ m_depth_omega(α)` ⇒ reject. Gate (b)
///     — free-variable preservation (V3-NEW): every 2-functor M in
///     the diakrisis Axi-2 family preserves variable scope (M is
///     not allowed to rename free variables, only to wrap existing
///     term structure with metaisation), so `free_vars(M_α) ≠
///     free_vars(α)` ⇒ reject; this catches the "M renames vars"
///     failure class V2's depth check missed. Gate (c) —
///     β-normalisation invariance (V3-NEW): the naturality square
///     commutes up to β-equivalence; if `definitional_eq(M_α, α)`
///     holds the M-action is a β-redex that normalises away — the
///     V1 identity sub-case extends to any term sharing a normal
///     form. When all three gates pass, V3-incremental accepts;
///     full *sufficient* witness construction (σ_α from Code_S
///     morphism + π_α from Perform_{ε_math} naturality through
///     axiom A-3) is the V3-final step still tracked under #181.
///     V3-incremental rejects strictly more terms than V2 (every
///     V2 reject stays rejected, plus terms clearing the depth
///     gate but failing (b) or (c)).
///
///   • Any other pair (including `(EpsilonOf(_), AlphaOf(t))`
///     where `t` is not an `EpsilonOf`) is rejected with
///     `EpsMuNaturalityFailed`.
///
/// **Soundness**: each V3 gate is a logical consequence of the
/// τ-witness existence; rejecting on any failure rejects no term
/// that *could* have a witness — the rejection is one-sided. The
/// V3-final sufficient construction will *not* widen this
/// accept set; it will only *prove* sufficiency for the
/// V3-incremental accept set.
///
/// **Completeness gap to V3-final**: V3-incremental still
/// over-accepts on terms passing all three gates without admitting
/// a Code_S/Perform_{ε_math} witness. The over-acceptance is
/// confined to terms whose M-action is structurally consistent
/// with naturality but whose witness construction blows up for
/// type-theoretic reasons (e.g. positivity violations in the
/// witness elaboration). V3-final will plug this remaining gap.
///
/// Decidability: V3-incremental is decidable in time linear in
/// the term sizes plus the cost of `definitional_eq` (β-normal
/// form computation, which is normalising-cache-backed —
/// amortised constant after first normalise per term).
pub fn check_eps_mu_coherence(
    lhs: &CoreTerm,
    rhs: &CoreTerm,
    context: &str,
) -> Result<(), KernelError> {
    // Degenerate identity-naturality square: structural equality
    // covers both the referential and the deep-equal case (CoreTerm
    // derives PartialEq).
    if lhs == rhs {
        return Ok(());
    }
    match (lhs, rhs) {
        // Canonical naturality-square shape with the V1 tightening:
        // the inner of `AlphaOf` must itself be an `EpsilonOf`. This
        // catches malformed pairs like (EpsilonOf(_), AlphaOf(Var(_))).
        (CoreTerm::EpsilonOf(m_alpha), CoreTerm::AlphaOf(inner_rhs)) => {
            match inner_rhs.as_ref() {
                CoreTerm::EpsilonOf(alpha_rhs) => {
                    // V1 sufficient witness: identity-functor case
                    // (`M = id` ⇒ `M_α = α`). When `m_alpha == α_rhs`
                    // structurally, the naturality square commutes
                    // trivially.
                    if m_alpha.as_ref() == alpha_rhs.as_ref() {
                        return Ok(());
                    }

                    // V3-NEW gate (c): β-normalisation invariance.
                    // Extends the V1 identity sub-case from
                    // structural equality to definitional equality,
                    // catching M-actions that are β-redexes
                    // normalising back to α.
                    if definitional_eq(m_alpha.as_ref(), alpha_rhs.as_ref()) {
                        return Ok(());
                    }

                    // V2 gate (a): modal-depth preservation
                    // pre-condition. The natural equivalence
                    // τ : ε ∘ M ≃ A ∘ ε is depth-preserving;
                    // depth mismatch ⇒ no witness ⇒ reject.
                    let lhs_rank = m_depth_omega(m_alpha.as_ref());
                    let rhs_rank = m_depth_omega(alpha_rhs.as_ref());
                    if lhs_rank != rhs_rank {
                        // V2.5 — embed the depth mismatch in the
                        // diagnostic context so callers can post-
                        // mortem the rejection without re-running
                        // m_depth_omega themselves.
                        return Err(KernelError::EpsMuNaturalityFailed {
                            context: Text::from(format!(
                                "{}: depth mismatch md^ω(M_α)={} vs md^ω(α)={}",
                                context,
                                lhs_rank.render(),
                                rhs_rank.render(),
                            )),
                        });
                    }

                    // V3-NEW gate (b): free-variable preservation.
                    // The Diakrisis Axi-2 2-functor M preserves
                    // variable scope; introducing or removing
                    // free variables would violate naturality.
                    // BTreeSet equality is order-independent and
                    // O(n log n) in the number of free names —
                    // negligible relative to definitional_eq.
                    let lhs_fvs = free_vars(m_alpha.as_ref());
                    let rhs_fvs = free_vars(alpha_rhs.as_ref());
                    if lhs_fvs != rhs_fvs {
                        // Compute the asymmetric difference for the
                        // diagnostic so the user can see exactly
                        // which variables drifted. We sort + join
                        // for stable output.
                        let extra_lhs: Vec<&str> = lhs_fvs
                            .difference(&rhs_fvs)
                            .map(|t| t.as_str())
                            .collect();
                        let extra_rhs: Vec<&str> = rhs_fvs
                            .difference(&lhs_fvs)
                            .map(|t| t.as_str())
                            .collect();
                        return Err(KernelError::EpsMuNaturalityFailed {
                            context: Text::from(format!(
                                "{}: free-var mismatch — \
                                 in M_α only [{}], in α only [{}]",
                                context,
                                extra_lhs.join(", "),
                                extra_rhs.join(", "),
                            )),
                        });
                    }

                    // All V3-incremental necessary conditions pass.
                    // V3-final sufficient witness (σ_α / π_α) is
                    // still pending (#181); accept conservatively.
                    Ok(())
                }
                _ => Err(KernelError::EpsMuNaturalityFailed {
                    context: Text::from(context),
                }),
            }
        }
        // Anything else: V1 cannot certify; record the context.
        _ => Err(KernelError::EpsMuNaturalityFailed {
            context: Text::from(context),
        }),
    }
}

// =============================================================================
// V3-final: τ-witness construction with explicit Diakrisis A-3 bridge admit.
// =============================================================================

/// V3-final naturality witness — the audit-trail-aware promotion of
/// [`check_eps_mu_coherence`].
///
/// V3-incremental decides necessary conditions structurally
/// (depth preservation, free-variable preservation, β-normalisation
/// invariance). V3-final additionally surfaces the σ_α / π_α
/// sufficient-witness construction as an explicit
/// [`crate::diakrisis_bridge::BridgeId::EpsMuTauWitness`] admit when
/// the canonical naturality square is non-trivial — i.e. when the
/// pair clears all V3-incremental gates but isn't structurally /
/// β-equivalently the identity case.
///
/// **Strictly stronger than V3-incremental**: every pair the V3-
/// incremental algorithm admits is also admitted by V3-final with
/// either an empty audit (identity / β-equivalent cases) or with a
/// single `EpsMuTauWitness` admit recorded (non-identity cases that
/// rely on Diakrisis A-3).
///
/// **Soundness invariant**: V3-final never widens V3-incremental's
/// accept set — the bridge admit only documents WHICH witness
/// construction is being relied on, not whether the rule fires. A
/// pair that V3-incremental rejects (depth / free-var / β-norm
/// gate failure) is rejected by V3-final too.
///
/// **V3 promotion path**: when Diakrisis A-3 lands as a structural
/// algorithm (Code_S morphism + Perform_{ε_math} naturality),
/// `admit_eps_mu_tau_witness`'s body is replaced with the actual
/// witness computation. Every previously-admitted call site mutates
/// from `audit = {EpsMuTauWitness}` to `audit = {}`, monotonically
/// shrinking the trusted boundary.
pub fn check_eps_mu_coherence_v3_final(
    lhs: &CoreTerm,
    rhs: &CoreTerm,
    context: &str,
) -> Result<BridgeAudit, KernelError> {
    // Step 1: run V3-incremental gates. Any failure surfaces directly.
    check_eps_mu_coherence(lhs, rhs, context)?;

    // Step 2: classify the admitted pair to decide whether the
    // sufficient-witness construction (σ_α / π_α) was needed.
    let mut audit = BridgeAudit::new();

    // Identity sub-case: structural equality on the entire pair, or
    // canonical naturality-square shape with structurally-equal inner
    // M_α and α_rhs. These are decidable without invoking A-3.
    if lhs == rhs {
        return Ok(audit);
    }
    if let (CoreTerm::EpsilonOf(m_alpha), CoreTerm::AlphaOf(inner_rhs)) = (lhs, rhs) {
        if let CoreTerm::EpsilonOf(alpha_rhs) = inner_rhs.as_ref() {
            // V1 identity sub-case (structural).
            if m_alpha.as_ref() == alpha_rhs.as_ref() {
                return Ok(audit);
            }
            // V3-NEW gate (c) — β-equivalent identity sub-case.
            if definitional_eq(m_alpha.as_ref(), alpha_rhs.as_ref()) {
                return Ok(audit);
            }
            // Non-identity case: V3-incremental cleared all gates
            // (depth + free-vars), but the σ_α / π_α witness
            // construction itself is preprint-blocked. Record A-3.
            admit_eps_mu_tau_witness(&mut audit, context, lhs);
            return Ok(audit);
        }
    }

    // V3-incremental admitted via some path we don't recognise here
    // — surface as A-3 admit conservatively (one-sided over-admit
    // becomes one-sided over-record; safer than silent decidable).
    admit_eps_mu_tau_witness(&mut audit, context, lhs);
    Ok(audit)
}

#[cfg(test)]
mod v3_final_tests {
    use super::*;
    use crate::diakrisis_bridge::BridgeId;
    use verum_common::Heap;

    fn var(n: &str) -> CoreTerm {
        CoreTerm::Var(Text::from(n))
    }

    fn eps(t: CoreTerm) -> CoreTerm {
        CoreTerm::EpsilonOf(Heap::new(t))
    }

    fn alpha(t: CoreTerm) -> CoreTerm {
        CoreTerm::AlphaOf(Heap::new(t))
    }

    #[test]
    fn v3_final_admits_structural_identity_with_empty_audit() {
        let f = var("F");
        let audit = check_eps_mu_coherence_v3_final(&f, &f, "structural id").unwrap();
        assert!(audit.is_decidable(),
            "structural identity must be decidable in V3-final");
    }

    #[test]
    fn v3_final_admits_canonical_naturality_identity_with_empty_audit() {
        // (EpsilonOf(F), AlphaOf(EpsilonOf(F))) — V1 identity sub-case.
        let f = var("F");
        let lhs = eps(f.clone());
        let rhs = alpha(eps(f.clone()));
        let audit = check_eps_mu_coherence_v3_final(&lhs, &rhs, "K-Eps-Mu identity")
            .unwrap();
        assert!(audit.is_decidable(),
            "canonical identity sub-case must be decidable");
    }

    #[test]
    fn v3_final_records_a_3_for_non_identity_canonical_pair() {
        // Non-identity canonical naturality square that clears V3-
        // incremental gates: M_α and α have different structure but
        // matching depth + free-vars + non-β-equal. Since variables
        // are atomic with depth 0 and no free-vars beyond the var
        // name itself, we craft (EpsilonOf(F), AlphaOf(EpsilonOf(G)))
        // where F != G but same structural shape.
        //
        // Both F and G are atomic Var with depth 0 — passes V3-incremental
        // gate (a). free_vars(F) = {F}, free_vars(G) = {G} — they DIFFER,
        // so V3-incremental REJECTS (gate b). This means it shouldn't
        // reach the bridge admit path. Let's instead use F twice on lhs
        // then a structurally-same shape wrapping a β-redex.
        //
        // Actually the cleanest non-identity case that clears V3-
        // incremental is when M_α and α_rhs have the SAME free vars +
        // SAME depth but are NOT structurally equal AND NOT β-equal.
        // Constructing such a pair requires careful term shaping.
        //
        // Simplest: M_α = App(Var("id"), Var("F"))  and  α_rhs = App(Var("id"), Var("F")).
        // But those ARE structurally equal — caught by identity case.
        //
        // Use App(F, x) on one side, x on the other with same fvs:
        // m_α = App(Var("F"), Var("x"))  ; depth 0, fvs {F, x}
        // α_rhs needs same fvs {F, x} and same depth.
        //   α_rhs = App(Var("x"), Var("F"))  ; depth 0, fvs {F, x}, distinct shape.
        //
        // Not β-equal (no Lam to reduce). V3-incremental admits.
        // V3-final routes through the bridge admit.
        let m_alpha = CoreTerm::App(Heap::new(var("F")), Heap::new(var("x")));
        let alpha_rhs = CoreTerm::App(Heap::new(var("x")), Heap::new(var("F")));
        let lhs = eps(m_alpha);
        let rhs = alpha(eps(alpha_rhs));
        let audit = check_eps_mu_coherence_v3_final(&lhs, &rhs, "non-identity")
            .unwrap();
        assert!(!audit.is_decidable(),
            "non-identity case must record an A-3 admit");
        let admits = audit.admits();
        assert_eq!(admits.len(), 1);
        assert_eq!(admits[0].bridge, BridgeId::EpsMuTauWitness);
    }

    #[test]
    fn v3_final_rejects_depth_mismatch() {
        // V3-incremental rejects on depth mismatch — V3-final must
        // surface the same rejection (V3-final is strictly stronger,
        // not strictly weaker).
        // ModalBox bumps depth by 1; if lhs has Box and rhs doesn't,
        // depths differ → V3-incremental rejects via gate (a).
        let f = var("F");
        let lhs = eps(CoreTerm::ModalBox(Heap::new(f.clone())));
        let rhs = alpha(eps(f));
        let result = check_eps_mu_coherence_v3_final(&lhs, &rhs, "depth-mismatch");
        assert!(result.is_err(),
            "depth-mismatch must be rejected by V3-final too");
    }

    #[test]
    fn v3_final_rejects_free_var_mismatch() {
        // Same depth (both depth-0), DIFFERENT free vars → gate (b) rejects.
        // m_α = Var("F"), α_rhs = Var("G")  — fvs differ.
        let lhs = eps(var("F"));
        let rhs = alpha(eps(var("G")));
        let result = check_eps_mu_coherence_v3_final(&lhs, &rhs, "free-var-mismatch");
        assert!(result.is_err(),
            "free-var-mismatch must be rejected by V3-final too");
    }

    #[test]
    fn v3_final_rejects_malformed_pair() {
        // (EpsilonOf(F), AlphaOf(Var("G")))  — inner of AlphaOf is
        // not an EpsilonOf, so V3-incremental rejects shape.
        let lhs = eps(var("F"));
        let rhs = alpha(var("G"));
        let result = check_eps_mu_coherence_v3_final(&lhs, &rhs, "malformed");
        assert!(result.is_err());
    }

    #[test]
    fn v3_final_audit_records_one_admit_per_bridge_invocation() {
        // Audit dedup: the same callsite invoking the same bridge
        // logs once. We invoke the function twice on the same pair
        // — each invocation gets its OWN BridgeAudit, so the
        // single-context-single-admit invariant is per-call.
        let m_alpha = CoreTerm::App(Heap::new(var("F")), Heap::new(var("x")));
        let alpha_rhs = CoreTerm::App(Heap::new(var("x")), Heap::new(var("F")));
        let lhs = eps(m_alpha);
        let rhs = alpha(eps(alpha_rhs));
        let audit1 = check_eps_mu_coherence_v3_final(&lhs, &rhs, "ctx-A").unwrap();
        let audit2 = check_eps_mu_coherence_v3_final(&lhs, &rhs, "ctx-B").unwrap();
        assert_eq!(audit1.admits().len(), 1);
        assert_eq!(audit2.admits().len(), 1);
        assert_ne!(audit1.admits()[0].context.as_str(),
                   audit2.admits()[0].context.as_str(),
                   "different contexts must produce distinct audit entries");
    }
}
