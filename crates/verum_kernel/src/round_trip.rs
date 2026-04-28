//! `K-Round-Trip` kernel rule — OC/DC translation round-trip
//! admission (Theorem 108.T / Theorem 16.10).
//!
//! Pre-this-module the rule existed only as a `KernelRule::KRoundTrip`
//! taxonomy entry (`proof_tree.rs:771`) at V0 stage with no actual
//! rule logic. This module ships the V0/V1 admission gate for the
//! identity-functor and definitional-equality cases that are the
//! load-bearing instances for the AC/OC duality (MSFS Theorem 10.4)
//! flagship reasoning.
//!
//! ## What the rule certifies
//!
//! Given an articulation `α : Articulation` (carried as a `CoreTerm`),
//! `K-Round-Trip` certifies that
//!
//! ```text
//!     canonicalise(inverse(translate(α))) ≡ canonicalise(α)
//! ```
//!
//! holds — i.e. translating to the dual side and back, then
//! canonicalising, recovers the same canonical class.
//!
//! ## V0 (this version) — identity-functor case
//!
//! When the articulation is a syntactic-self-enactment
//! `epsilon(F)` (carried as `CoreTerm::EpsilonOf(F)`), the round-trip
//! is literally identity:
//!
//! ```text
//!     canonicalise(alpha(epsilon(epsilon(F))))
//!         ≡ canonicalise(epsilon(F))    [K-Adj-Unit: α∘ε = id]
//! ```
//!
//! V0 admits this case via structural equality on the term shape.
//!
//! ## V1 (this version) — definitional-equality round-trip
//!
//! Extends V0 to admit any pair `(α, α')` whose β-/ι-/δ-normal forms
//! coincide, using [`crate::support::definitional_eq`]. This catches
//! M-actions that are β-redexes normalising back to the same
//! articulation under the K-Adj-Unit/Counit definitional rules.
//!
//! ## V2 (preprint-blocked) — full canonicalize algorithm
//!
//! The universal case (`canonicalise(inverse(translate(α)))` for an
//! arbitrary `α`) requires the Diakrisis Theorem 16.10 algorithmic
//! content for canonicalize. Until that lands, V2 is deferred and
//! callers requiring round-trip admission for non-identity `α` must
//! supply an explicit framework axiom.

use verum_common::Text;

use crate::CoreTerm;
use crate::KernelError;
use crate::support::definitional_eq;

/// `K-Round-Trip` admission rule (V0/V1).
///
/// Accepts:
///
///   * **V0 — identity sub-case.** `lhs == rhs` structurally — the
///     trivial round-trip on a syntactic-self-enactment.
///   * **V0 — `EpsilonOf` symmetric pair.** `lhs = EpsilonOf(F)` and
///     `rhs = EpsilonOf(F)` with structurally-equal `F`.
///   * **V0 — `AlphaOf(EpsilonOf(F))` ↔ identity.** When one side is
///     `AlphaOf(EpsilonOf(F))` and the other is `F`, K-Adj-Unit
///     ensures the round-trip is identity.
///   * **V1 — definitional-equality round-trip.** `lhs` and `rhs`
///     reduce to the same β-/ι-/δ-normal form.
///
/// Rejects everything else with [`KernelError::RoundTripFailed`]
/// tagged with `context`. V2 (preprint-blocked) will extend the
/// admit-set with the full canonicalize algorithm; V0/V1 are
/// strictly necessary conditions, never silent-accept.
///
/// Context: typically a human-readable label describing the
/// callsite (e.g. `"AC/OC duality at Theorem 10.4"`), surfaced in
/// the rejection diagnostic.
pub fn check_round_trip(
    lhs: &CoreTerm,
    rhs: &CoreTerm,
    context: &str,
) -> Result<(), KernelError> {
    // V0 — structural equality fast-path. Catches the trivial
    // `α == α` round-trip without invoking normalize.
    if lhs == rhs {
        return Ok(());
    }

    // V0 — AlphaOf(EpsilonOf(x)) on one side ≡ x on the other side
    // (K-Adj-Unit: α∘ε = id). Exact-shape match — no β-reduction.
    match (lhs, rhs) {
        (CoreTerm::AlphaOf(inner), other) | (other, CoreTerm::AlphaOf(inner)) => {
            if let CoreTerm::EpsilonOf(payload) = inner.as_ref() {
                if payload.as_ref() == other {
                    return Ok(());
                }
                // V1 lift on the inner: definitional equality.
                if definitional_eq(payload.as_ref(), other) {
                    return Ok(());
                }
            }
        }
        _ => {}
    }

    // V0 — EpsilonOf(AlphaOf(x)) on one side ≡ x on the other side
    // for the IMAGE-of-syntactic-self case (K-Adj-Counit: ε∘α ≃ id
    // on syntactic self-enactments). Exact shape + V1 definitional.
    match (lhs, rhs) {
        (CoreTerm::EpsilonOf(inner), other) | (other, CoreTerm::EpsilonOf(inner)) => {
            if let CoreTerm::AlphaOf(payload) = inner.as_ref() {
                if payload.as_ref() == other {
                    return Ok(());
                }
                if definitional_eq(payload.as_ref(), other) {
                    return Ok(());
                }
            }
        }
        _ => {}
    }

    // V1 — definitional-equality round-trip. Catches β-/ι-/δ-equal
    // pairs that aren't structurally identical. Same uniqueness-
    // up-to-α invariant as `verify_full` (post-M-VVA Sub-2.3).
    if definitional_eq(lhs, rhs) {
        return Ok(());
    }

    // V2-pending: universal canonicalize algorithm (Diakrisis 16.10).
    // Until then the rule cannot certify non-identity round-trips.
    Err(KernelError::RoundTripFailed {
        context: Text::from(context),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_common::Heap;

    fn var(name: &str) -> CoreTerm {
        CoreTerm::Var(Text::from(name))
    }

    #[test]
    fn round_trip_accepts_structural_identity() {
        let alpha = var("F");
        assert!(check_round_trip(&alpha, &alpha, "test").is_ok());
    }

    #[test]
    fn round_trip_accepts_alpha_of_epsilon_of_x_vs_x() {
        // AlphaOf(EpsilonOf(F)) ≡ F via K-Adj-Unit.
        let f = var("F");
        let aef = CoreTerm::AlphaOf(Heap::new(CoreTerm::EpsilonOf(Heap::new(f.clone()))));
        assert!(check_round_trip(&aef, &f, "K-Adj-Unit").is_ok());
        // Symmetric: x on one side, AlphaOf(EpsilonOf(x)) on the other.
        assert!(check_round_trip(&f, &aef, "K-Adj-Unit symmetric").is_ok());
    }

    #[test]
    fn round_trip_accepts_epsilon_of_alpha_of_x_vs_x() {
        // EpsilonOf(AlphaOf(F)) ≡ F via K-Adj-Counit (on syntactic
        // self-enactments).
        let f = var("F");
        let eaf = CoreTerm::EpsilonOf(Heap::new(CoreTerm::AlphaOf(Heap::new(f.clone()))));
        assert!(check_round_trip(&eaf, &f, "K-Adj-Counit").is_ok());
        assert!(check_round_trip(&f, &eaf, "K-Adj-Counit symmetric").is_ok());
    }

    #[test]
    fn round_trip_accepts_definitionally_equal_pair() {
        // (λx. x) F  ≡_β  F. Round-trip across β-redex must be admitted.
        let f = var("F");
        let beta_redex = CoreTerm::App(
            Heap::new(CoreTerm::Lam {
                binder: Text::from("x"),
                domain: Heap::new(CoreTerm::Universe(crate::UniverseLevel::Concrete(0))),
                body: Heap::new(CoreTerm::Var(Text::from("x"))),
            }),
            Heap::new(f.clone()),
        );
        assert!(check_round_trip(&beta_redex, &f, "β-equal").is_ok());
    }

    #[test]
    fn round_trip_rejects_distinct_atoms() {
        let alpha = var("alpha");
        let beta = var("beta");
        let err = check_round_trip(&alpha, &beta, "distinct").unwrap_err();
        assert!(matches!(err, KernelError::RoundTripFailed { .. }));
    }

    #[test]
    fn round_trip_rejects_alpha_of_distinct() {
        // AlphaOf(EpsilonOf(F)) vs G — F != G structurally.
        let aef = CoreTerm::AlphaOf(Heap::new(CoreTerm::EpsilonOf(Heap::new(var("F")))));
        let g = var("G");
        let err = check_round_trip(&aef, &g, "AlphaOf-mismatch").unwrap_err();
        assert!(matches!(err, KernelError::RoundTripFailed { .. }));
    }
}
