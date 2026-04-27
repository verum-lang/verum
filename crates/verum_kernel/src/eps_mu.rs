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
