//! VFE-1 K-Eps-Mu kernel rule — split per #198.
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
use crate::{CoreTerm, KernelError};

/// VFE-1 V0/V1/V2 — `K-Eps-Mu` kernel rule entry point.
///
/// V0 shipped a permissive skeleton that accepted any pair
/// `(EpsilonOf(_), AlphaOf(_))`. V1 tightened the shape check; V2
/// (this revision) adds the **modal-depth preservation
/// pre-condition** for non-identity M:
///
///   • `(EpsilonOf(M_α), AlphaOf(EpsilonOf(α)))` is the canonical
///     naturality-square shape per Proposition 5.1 / Corollary 5.10.
///     The inner of `AlphaOf` MUST be an `EpsilonOf` constructor.
///
///   • `(t, t)` (structurally equal) is the degenerate identity-
///     naturality square — always accepted.
///
///   • For the M = id sub-case (`m_alpha == α_rhs` structurally),
///     accept directly: identity-functor naturality commutes
///     trivially.
///
///   • For the non-identity-M case (`m_alpha != α_rhs`), V2 checks
///     a **necessary condition** for the τ-witness to exist: the
///     natural-equivalence τ : ε ∘ M ≃ A ∘ ε is an (∞,1)-
///     categorical equivalence, hence depth-preserving. So
///     `m_depth_omega(M_α) ≠ m_depth_omega(α)` ⇒ no τ-witness can
///     possibly exist ⇒ reject. When depths agree, V2 still
///     conservatively accepts — the depth check is a *necessary*
///     condition, not a sufficient one. Sufficient witness
///     construction (σ_α / π_α) is the V3 work tracked under #181.
///
///   • Any other pair (including `(EpsilonOf(_), AlphaOf(t))` where
///     `t` is not itself an `EpsilonOf`) is rejected with
///     `EpsMuNaturalityFailed`.
///
/// **What V2 does *not* yet check** (still V3 / #181 work):
///
///   • The explicit τ-witness construction (σ_α from Code_S morphism
///     + π_α from Perform_{ε_math} naturality through axiom A-3).
///   • V2's depth-equality is necessary but not sufficient: two
///     terms can share modal depth without admitting a τ-witness
///     (the witness construction may fail for type-theoretic
///     reasons orthogonal to depth). V3 will add the σ_α/π_α
///     construction; V2 just rules out the obvious depth-mismatch
///     impossibility.
///
/// Decidability: the check is *semi-decidable* in general (per the
/// structure-recursion argument that backs Theorem 16.6). For
/// finitely-axiomatised articulations the check reduces to round-trip
/// 16.10 and is decidable in single-exponential time. V1's shape
/// check terminates in linear time on the term sizes.
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
                        Ok(())
                    } else {
                        // V2 increment: modal-depth preservation
                        // pre-condition for non-identity M. The
                        // canonical natural-equivalence
                        // τ : ε ∘ M ≃ A ∘ ε is depth-preserving;
                        // a depth mismatch precludes any τ-witness,
                        // so reject. Depth match ⇒ V2 still
                        // conservatively accepts pending the V3
                        // (#181) full τ-witness construction.
                        let lhs_rank = m_depth_omega(m_alpha.as_ref());
                        let rhs_rank = m_depth_omega(alpha_rhs.as_ref());
                        if lhs_rank == rhs_rank {
                            Ok(())
                        } else {
                            Err(KernelError::EpsMuNaturalityFailed {
                                context: Text::from(context),
                            })
                        }
                    }
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
