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
//! ## V2 — universal canonicalize with Diakrisis bridge admits
//!
//! V2 ships [`canonical_form`], a normalize-to-fixed-point algorithm
//! that walks every CoreTerm constructor applying:
//!
//!   * **K-Adj-Unit** — `AlphaOf(EpsilonOf(x)) → x`.
//!   * **K-Adj-Counit** — `EpsilonOf(AlphaOf(x)) → x` (where
//!     applicable on syntactic-self-enactments).
//!   * **K-Refine fold** — `Refine(Refine(B, p₁), p₂) → Refine(B,
//!     p₁ ∧ p₂)` for nested refinements with identical binders.
//!   * **K-Modal-Idem** — `ModalBox(ModalBox(x)) → ModalBox(x)`,
//!     `ModalDiamond(ModalDiamond(x)) → ModalDiamond(x)` (S5).
//!   * **K-Cohesive-Idem** — `Shape(Shape(x)) → Shape(x)`,
//!     `Flat(Flat(x)) → Flat(x)`, `Sharp(Sharp(x)) → Sharp(x)`.
//!   * **K-β/η/ι/δ** — delegated to [`crate::support::normalize`].
//!
//! The full Diakrisis Theorem 16.10 confluence claim is preprint-
//! blocked; V2 surfaces it as an explicit
//! [`crate::diakrisis_bridge::BridgeId::ConfluenceOfModalRewrite`]
//! admit recorded in the [`BridgeAudit`] returned by
//! [`check_round_trip_v2`]. V3 will discharge the bridge once the
//! preprint result lands as a structural algorithm.
//!
//! Callers that need legacy V0/V1-only behaviour keep using
//! [`check_round_trip`]; callers that opt into V2's wider admit set
//! pay the audit-trail introspection cost in exchange for accepting
//! a substantially larger fragment.

use verum_common::{Heap, Text};

use crate::CoreTerm;
use crate::KernelError;
use crate::diakrisis_bridge::{BridgeAudit, admit_confluence_of_modal_rewrite};
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

// =============================================================================
// V2: Universal canonicalize + audit-trail-aware round-trip.
// =============================================================================

/// Bound on the canonicalize-to-fixed-point iteration count.  Picked
/// so that under typical kernel input the loop terminates in 4–6
/// rounds; pathological adversarial input that bumps against this
/// limit causes [`check_round_trip_v2`] to invoke the
/// [`BridgeId::ConfluenceOfModalRewrite`] admit and surface a
/// non-decidable audit trail.
const CANONICALIZE_ITERATION_BUDGET: u32 = 64;

/// V2 universal canonicalize: walk every CoreTerm constructor applying
/// the K-rule rewrites, then β-/η-/ι-/δ-normalize via
/// [`crate::support::normalize`], iterating until a fixed point or
/// the iteration budget is exhausted.
///
/// The audit trail is mutated when a rewrite class can't be applied
/// without invoking a [`BridgeId`] admit. A decidable run leaves the
/// audit trail empty.
///
/// `context` is propagated into every bridge admit so external
/// reporters can attribute admits back to a callsite.
pub fn canonical_form(
    term: &CoreTerm,
    audit: &mut BridgeAudit,
    context: &str,
) -> CoreTerm {
    let mut current = term.clone();
    for _ in 0..CANONICALIZE_ITERATION_BUDGET {
        let rewritten = rewrite_one_pass(&current, audit, context);
        let normalized = crate::support::normalize(&rewritten);
        if normalized == current {
            return normalized;
        }
        current = normalized;
    }
    // Budget exhausted — admit the confluence bridge: under the
    // Diakrisis 16.10 result, the iteration would converge; we
    // record the admit and return whatever fixed-pointwise close-
    // to-stable form we reached.
    admit_confluence_of_modal_rewrite(audit, context, &current)
}

/// Single rewrite pass over the entire term: applies every K-rule
/// rewrite once at every position, returning the new term. Idempotent
/// under composition with [`crate::support::normalize`] modulo the
/// confluence bridge.
fn rewrite_one_pass(
    term: &CoreTerm,
    audit: &mut BridgeAudit,
    context: &str,
) -> CoreTerm {
    match term {
        // K-Adj-Unit: AlphaOf(EpsilonOf(x)) → x.
        CoreTerm::AlphaOf(inner) => {
            let inner_rw = rewrite_one_pass(inner.as_ref(), audit, context);
            if let CoreTerm::EpsilonOf(payload) = &inner_rw {
                return rewrite_one_pass(payload.as_ref(), audit, context);
            }
            CoreTerm::AlphaOf(Heap::new(inner_rw))
        }

        // K-Adj-Counit: EpsilonOf(AlphaOf(x)) → x.
        CoreTerm::EpsilonOf(inner) => {
            let inner_rw = rewrite_one_pass(inner.as_ref(), audit, context);
            if let CoreTerm::AlphaOf(payload) = &inner_rw {
                return rewrite_one_pass(payload.as_ref(), audit, context);
            }
            CoreTerm::EpsilonOf(Heap::new(inner_rw))
        }

        // K-Refine V3 fold (NEW, was V2 stub):
        //
        //   Refine(Refine(B, x: p₁), y: p₂) → Refine(B, y: p₁[x↦y] ∧ p₂)
        //
        // Now decidable end-to-end via support::fold_refine_of_refine,
        // which handles the alpha-rename when the inner and outer
        // binders differ. Iteration to fixed point happens at the
        // outer canonical_form layer — a K-level-N nested Refine
        // collapses in N-1 outer iterations.
        CoreTerm::Refine { base, binder, predicate } => {
            let base_rw = rewrite_one_pass(base.as_ref(), audit, context);
            let pred_rw = rewrite_one_pass(predicate.as_ref(), audit, context);
            let candidate = CoreTerm::Refine {
                base: Heap::new(base_rw),
                binder: binder.clone(),
                predicate: Heap::new(pred_rw),
            };
            // V3 fold: if the rewritten base is itself a Refine, fuse.
            crate::support::fold_refine_of_refine(&candidate)
                .unwrap_or(candidate)
        }

        // K-Modal-Idem (S5): ModalBox(ModalBox(x)) → ModalBox(x),
        // ModalDiamond(ModalDiamond(x)) → ModalDiamond(x).
        CoreTerm::ModalBox(inner) => {
            let inner_rw = rewrite_one_pass(inner.as_ref(), audit, context);
            if matches!(inner_rw, CoreTerm::ModalBox(_)) {
                return inner_rw;
            }
            CoreTerm::ModalBox(Heap::new(inner_rw))
        }
        CoreTerm::ModalDiamond(inner) => {
            let inner_rw = rewrite_one_pass(inner.as_ref(), audit, context);
            if matches!(inner_rw, CoreTerm::ModalDiamond(_)) {
                return inner_rw;
            }
            CoreTerm::ModalDiamond(Heap::new(inner_rw))
        }

        // K-Cohesive-Idem: Shape/Flat/Sharp idempotency on their
        // adjunction side. ∫∫ ≡ ∫, ♭♭ ≡ ♭, ♯♯ ≡ ♯ for the typical
        // cohesive setup — Schreiber DCCT §3.1.
        CoreTerm::Shape(inner) => {
            let inner_rw = rewrite_one_pass(inner.as_ref(), audit, context);
            if matches!(inner_rw, CoreTerm::Shape(_)) {
                return inner_rw;
            }
            CoreTerm::Shape(Heap::new(inner_rw))
        }
        CoreTerm::Flat(inner) => {
            let inner_rw = rewrite_one_pass(inner.as_ref(), audit, context);
            if matches!(inner_rw, CoreTerm::Flat(_)) {
                return inner_rw;
            }
            CoreTerm::Flat(Heap::new(inner_rw))
        }
        CoreTerm::Sharp(inner) => {
            let inner_rw = rewrite_one_pass(inner.as_ref(), audit, context);
            if matches!(inner_rw, CoreTerm::Sharp(_)) {
                return inner_rw;
            }
            CoreTerm::Sharp(Heap::new(inner_rw))
        }

        // Recurse through compound constructors that don't have a
        // V2 surface rewrite of their own.
        CoreTerm::App(f, a) => CoreTerm::App(
            Heap::new(rewrite_one_pass(f.as_ref(), audit, context)),
            Heap::new(rewrite_one_pass(a.as_ref(), audit, context)),
        ),
        CoreTerm::Pi { binder, domain, codomain } => CoreTerm::Pi {
            binder: binder.clone(),
            domain: Heap::new(rewrite_one_pass(domain.as_ref(), audit, context)),
            codomain: Heap::new(rewrite_one_pass(codomain.as_ref(), audit, context)),
        },
        CoreTerm::Lam { binder, domain, body } => CoreTerm::Lam {
            binder: binder.clone(),
            domain: Heap::new(rewrite_one_pass(domain.as_ref(), audit, context)),
            body: Heap::new(rewrite_one_pass(body.as_ref(), audit, context)),
        },
        CoreTerm::Sigma { binder, fst_ty, snd_ty } => CoreTerm::Sigma {
            binder: binder.clone(),
            fst_ty: Heap::new(rewrite_one_pass(fst_ty.as_ref(), audit, context)),
            snd_ty: Heap::new(rewrite_one_pass(snd_ty.as_ref(), audit, context)),
        },
        CoreTerm::Pair(a, b) => CoreTerm::Pair(
            Heap::new(rewrite_one_pass(a.as_ref(), audit, context)),
            Heap::new(rewrite_one_pass(b.as_ref(), audit, context)),
        ),
        CoreTerm::Fst(p) => {
            CoreTerm::Fst(Heap::new(rewrite_one_pass(p.as_ref(), audit, context)))
        }
        CoreTerm::Snd(p) => {
            CoreTerm::Snd(Heap::new(rewrite_one_pass(p.as_ref(), audit, context)))
        }
        CoreTerm::Refl(t) => {
            CoreTerm::Refl(Heap::new(rewrite_one_pass(t.as_ref(), audit, context)))
        }
        CoreTerm::PathTy { carrier, lhs, rhs } => CoreTerm::PathTy {
            carrier: Heap::new(rewrite_one_pass(carrier.as_ref(), audit, context)),
            lhs: Heap::new(rewrite_one_pass(lhs.as_ref(), audit, context)),
            rhs: Heap::new(rewrite_one_pass(rhs.as_ref(), audit, context)),
        },

        // Atomic constructors and unsupported variants pass through.
        _ => term.clone(),
    }
}

/// V2 round-trip: `canonical_form(lhs) ≡ canonical_form(rhs)` modulo
/// the audit trail. Returns `Ok(audit)` on admission with the trail
/// populated by every bridge admit invoked; `Err(...)` when even
/// the V2 universal algorithm can't admit the pair.
///
/// V2 is **strictly stronger** than [`check_round_trip`]: every pair
/// the V0/V1 algorithm admits is also admitted by V2 with an empty
/// audit trail. Pairs that V2 admits but V0/V1 rejects produce a
/// non-empty audit trail.
pub fn check_round_trip_v2(
    lhs: &CoreTerm,
    rhs: &CoreTerm,
    context: &str,
) -> Result<BridgeAudit, KernelError> {
    // V0/V1 fast path: try the structural rules first; if they
    // admit, the audit trail stays empty (decidable run).
    if check_round_trip(lhs, rhs, context).is_ok() {
        return Ok(BridgeAudit::new());
    }

    // V2 universal: canonicalize both sides and compare. Each side
    // shares the audit trail so admits accumulate across the pair.
    let mut audit = BridgeAudit::new();
    let lhs_canon = canonical_form(lhs, &mut audit, context);
    let rhs_canon = canonical_form(rhs, &mut audit, context);

    if lhs_canon == rhs_canon || definitional_eq(&lhs_canon, &rhs_canon) {
        return Ok(audit);
    }

    Err(KernelError::RoundTripFailed {
        context: Text::from(context),
    })
}

/// Diagnostic: enumerate all bridge admits that would be invoked
/// for a given term. Used by `verum audit --proof-honesty` to
/// surface every reliance on the preprint-blocked confluence claim.
pub fn enumerate_bridge_admits(term: &CoreTerm, context: &str) -> BridgeAudit {
    let mut audit = BridgeAudit::new();
    let _ = canonical_form(term, &mut audit, context);
    audit
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

    // -------------------------------------------------------------------------
    // V2 universal canonicalize tests
    // -------------------------------------------------------------------------

    #[test]
    fn canonical_form_collapses_alpha_epsilon_pair() {
        // canonical_form(AlphaOf(EpsilonOf(F))) → F  (K-Adj-Unit).
        let f = var("F");
        let aef = CoreTerm::AlphaOf(Heap::new(CoreTerm::EpsilonOf(Heap::new(f.clone()))));
        let mut audit = BridgeAudit::new();
        let canon = canonical_form(&aef, &mut audit, "test");
        assert_eq!(canon, f);
        assert!(audit.is_decidable(),
            "K-Adj-Unit collapse must be decidable; got {} admits", audit.admits().len());
    }

    #[test]
    fn canonical_form_collapses_epsilon_alpha_pair() {
        // canonical_form(EpsilonOf(AlphaOf(F))) → F  (K-Adj-Counit).
        let f = var("F");
        let eaf = CoreTerm::EpsilonOf(Heap::new(CoreTerm::AlphaOf(Heap::new(f.clone()))));
        let mut audit = BridgeAudit::new();
        let canon = canonical_form(&eaf, &mut audit, "test");
        assert_eq!(canon, f);
        assert!(audit.is_decidable());
    }

    #[test]
    fn canonical_form_collapses_modal_box_idempotent() {
        // ModalBox(ModalBox(F)) → ModalBox(F)  (S5 idempotency).
        let f = var("F");
        let bbf = CoreTerm::ModalBox(Heap::new(CoreTerm::ModalBox(Heap::new(f.clone()))));
        let bf = CoreTerm::ModalBox(Heap::new(f.clone()));
        let mut audit = BridgeAudit::new();
        let canon = canonical_form(&bbf, &mut audit, "test");
        assert_eq!(canon, bf);
        assert!(audit.is_decidable());
    }

    #[test]
    fn canonical_form_collapses_modal_diamond_idempotent() {
        let f = var("F");
        let ddf = CoreTerm::ModalDiamond(Heap::new(CoreTerm::ModalDiamond(Heap::new(f.clone()))));
        let df = CoreTerm::ModalDiamond(Heap::new(f.clone()));
        let mut audit = BridgeAudit::new();
        let canon = canonical_form(&ddf, &mut audit, "test");
        assert_eq!(canon, df);
        assert!(audit.is_decidable());
    }

    #[test]
    fn canonical_form_collapses_cohesive_shape_idempotent() {
        let f = var("F");
        let ssf = CoreTerm::Shape(Heap::new(CoreTerm::Shape(Heap::new(f.clone()))));
        let sf = CoreTerm::Shape(Heap::new(f.clone()));
        let mut audit = BridgeAudit::new();
        let canon = canonical_form(&ssf, &mut audit, "test");
        assert_eq!(canon, sf);
        assert!(audit.is_decidable());
    }

    #[test]
    fn canonical_form_collapses_flat_idempotent() {
        let f = var("F");
        let ffx = CoreTerm::Flat(Heap::new(CoreTerm::Flat(Heap::new(f.clone()))));
        let fx = CoreTerm::Flat(Heap::new(f.clone()));
        let mut audit = BridgeAudit::new();
        let canon = canonical_form(&ffx, &mut audit, "test");
        assert_eq!(canon, fx);
        assert!(audit.is_decidable());
    }

    #[test]
    fn canonical_form_collapses_sharp_idempotent() {
        let f = var("F");
        let shsh = CoreTerm::Sharp(Heap::new(CoreTerm::Sharp(Heap::new(f.clone()))));
        let sh = CoreTerm::Sharp(Heap::new(f.clone()));
        let mut audit = BridgeAudit::new();
        let canon = canonical_form(&shsh, &mut audit, "test");
        assert_eq!(canon, sh);
        assert!(audit.is_decidable());
    }

    #[test]
    fn canonical_form_recurses_into_app() {
        // App(AlphaOf(EpsilonOf(F)), x)  →  App(F, x)
        let f = var("F");
        let x = var("x");
        let aef = CoreTerm::AlphaOf(Heap::new(CoreTerm::EpsilonOf(Heap::new(f.clone()))));
        let app = CoreTerm::App(Heap::new(aef), Heap::new(x.clone()));
        let expected = CoreTerm::App(Heap::new(f.clone()), Heap::new(x.clone()));
        let mut audit = BridgeAudit::new();
        let canon = canonical_form(&app, &mut audit, "test");
        assert_eq!(canon, expected);
    }

    #[test]
    fn canonical_form_recurses_into_pi() {
        // Pi binder traversal — the rewrite reaches both domain and codomain.
        let f = var("F");
        let aef = CoreTerm::AlphaOf(Heap::new(CoreTerm::EpsilonOf(Heap::new(f.clone()))));
        let pi = CoreTerm::Pi {
            binder: Text::from("x"),
            domain: Heap::new(aef.clone()),
            codomain: Heap::new(aef),
        };
        let mut audit = BridgeAudit::new();
        let canon = canonical_form(&pi, &mut audit, "test");
        match canon {
            CoreTerm::Pi { domain, codomain, .. } => {
                assert_eq!(domain.as_ref(), &f);
                assert_eq!(codomain.as_ref(), &f);
            }
            other => panic!("expected Pi, got {:?}", other),
        }
    }

    #[test]
    fn check_round_trip_v2_admits_v0_pairs_with_empty_audit() {
        // Every pair the V0/V1 algorithm admits must also be admitted
        // by V2 with an EMPTY audit trail (decidable invariant).
        let f = var("F");
        let aef = CoreTerm::AlphaOf(Heap::new(CoreTerm::EpsilonOf(Heap::new(f.clone()))));
        let audit = check_round_trip_v2(&aef, &f, "K-Adj-Unit").unwrap();
        assert!(audit.is_decidable(),
            "V0/V1-decidable pair must produce empty V2 audit");
    }

    #[test]
    fn check_round_trip_v2_admits_modal_idempotent_pairs() {
        // ModalBox(ModalBox(F)) ≡ ModalBox(F) under V2 canonicalize.
        // V0/V1 reject this (no Modal-Idem rule); V2 admits decidably
        // because the rewrite is structural, not bridge-blocked.
        let f = var("F");
        let bbf = CoreTerm::ModalBox(Heap::new(CoreTerm::ModalBox(Heap::new(f.clone()))));
        let bf = CoreTerm::ModalBox(Heap::new(f.clone()));
        let audit = check_round_trip_v2(&bbf, &bf, "K-Modal-Idem").unwrap();
        assert!(audit.is_decidable());
    }

    #[test]
    fn check_round_trip_v2_rejects_distinct_atoms() {
        // Even V2 can't admit truly distinct atoms.
        let err = check_round_trip_v2(&var("alpha"), &var("beta"), "distinct")
            .unwrap_err();
        assert!(matches!(err, KernelError::RoundTripFailed { .. }));
    }

    #[test]
    fn enumerate_bridge_admits_returns_empty_for_decidable_terms() {
        // A term that V2 canonicalizes without invoking a bridge
        // produces an empty audit.
        let f = var("F");
        let aef = CoreTerm::AlphaOf(Heap::new(CoreTerm::EpsilonOf(Heap::new(f))));
        let audit = enumerate_bridge_admits(&aef, "test");
        assert!(audit.is_decidable(),
            "K-Adj-Unit reduction must not invoke a bridge");
    }

    #[test]
    fn canonical_form_idempotent_under_repeated_application() {
        // canonical_form(canonical_form(t)) == canonical_form(t).
        let f = var("F");
        let aef = CoreTerm::AlphaOf(Heap::new(CoreTerm::EpsilonOf(Heap::new(f.clone()))));
        let mut audit1 = BridgeAudit::new();
        let canon1 = canonical_form(&aef, &mut audit1, "test");
        let mut audit2 = BridgeAudit::new();
        let canon2 = canonical_form(&canon1, &mut audit2, "test");
        assert_eq!(canon1, canon2,
            "canonicalize must be idempotent");
    }

    // -------------------------------------------------------------------------
    // K-Refine V3 fold tests — refine-of-refine collapse via canonical_form.
    // -------------------------------------------------------------------------

    #[test]
    fn canonical_form_folds_refine_of_refine_same_binder() {
        // Refine(Refine(B, x: p), x: q)  →  Refine(B, x: p ∧ q)
        let b = var("Int");
        let p = var("p");
        let q = var("q");
        let inner = CoreTerm::Refine {
            base: Heap::new(b.clone()),
            binder: Text::from("x"),
            predicate: Heap::new(p.clone()),
        };
        let outer = CoreTerm::Refine {
            base: Heap::new(inner),
            binder: Text::from("x"),
            predicate: Heap::new(q.clone()),
        };
        let mut audit = BridgeAudit::new();
        let canon = canonical_form(&outer, &mut audit, "K-Refine V3 same-binder");
        match canon {
            CoreTerm::Refine { base, binder, predicate } => {
                assert_eq!(base.as_ref(), &b, "base must be the original Int");
                assert_eq!(binder.as_str(), "x");
                let (p_back, q_back) = crate::support::is_conjunction(predicate.as_ref())
                    .expect("predicate must be a conjunction");
                assert_eq!(p_back, &p);
                assert_eq!(q_back, &q);
            }
            other => panic!("expected single Refine, got {:?}", other),
        }
        assert!(audit.is_decidable(),
            "K-Refine V3 fold must be fully decidable (no bridge admit)");
    }

    #[test]
    fn canonical_form_folds_three_level_refine_chain() {
        // Refine(Refine(Refine(B, x: p), x: q), x: r)
        //   canonical iter 1:  Refine(Refine(B, x: p), x: q ∧ r)
        //   canonical iter 2:  Refine(B, x: p ∧ (q ∧ r))
        let b = var("Int");
        let l1 = CoreTerm::Refine {
            base: Heap::new(b.clone()),
            binder: Text::from("x"),
            predicate: Heap::new(var("p")),
        };
        let l2 = CoreTerm::Refine {
            base: Heap::new(l1),
            binder: Text::from("x"),
            predicate: Heap::new(var("q")),
        };
        let l3 = CoreTerm::Refine {
            base: Heap::new(l2),
            binder: Text::from("x"),
            predicate: Heap::new(var("r")),
        };
        let mut audit = BridgeAudit::new();
        let canon = canonical_form(&l3, &mut audit, "K-Refine V3 three-level");
        match &canon {
            CoreTerm::Refine { base, predicate, .. } => {
                assert_eq!(base.as_ref(), &b,
                    "three-level fold must collapse fully to a single Refine over the underlying base");
                // Predicate is some conjunction of p, q, r — check the
                // structural shape contains all three names.
                let pred_str = format!("{:?}", predicate.as_ref());
                assert!(pred_str.contains('p'));
                assert!(pred_str.contains('q'));
                assert!(pred_str.contains('r'));
            }
            other => panic!("three-level fold must produce single Refine, got {:?}", other),
        }
        assert!(audit.is_decidable());
    }

    #[test]
    fn canonical_form_folds_refine_with_alpha_rename() {
        // Refine(Refine(B, y: p(y)), x: q(x))
        //   →  Refine(B, x: p(x) ∧ q(x))   (inner-binder y renamed to x)
        let b = var("Int");
        let inner = CoreTerm::Refine {
            base: Heap::new(b.clone()),
            binder: Text::from("y"),
            predicate: Heap::new(CoreTerm::App(
                Heap::new(var("p")),
                Heap::new(var("y")),
            )),
        };
        let outer = CoreTerm::Refine {
            base: Heap::new(inner),
            binder: Text::from("x"),
            predicate: Heap::new(CoreTerm::App(
                Heap::new(var("q")),
                Heap::new(var("x")),
            )),
        };
        let mut audit = BridgeAudit::new();
        let canon = canonical_form(&outer, &mut audit, "K-Refine V3 alpha-rename");
        match canon {
            CoreTerm::Refine { base, binder, predicate } => {
                assert_eq!(base.as_ref(), &b);
                assert_eq!(binder.as_str(), "x");
                // Verify y was renamed to x by checking the conjunction
                // contains p(x), not p(y).
                let pred_str = format!("{:?}", predicate.as_ref());
                assert!(!pred_str.contains("\"y\""),
                    "inner binder y must have been alpha-renamed away: {pred_str}");
            }
            other => panic!("expected Refine, got {:?}", other),
        }
        assert!(audit.is_decidable());
    }

    #[test]
    fn canonical_form_preserves_single_refine() {
        // Single-level Refine is already canonical.
        let r = CoreTerm::Refine {
            base: Heap::new(var("Int")),
            binder: Text::from("x"),
            predicate: Heap::new(var("p")),
        };
        let mut audit = BridgeAudit::new();
        let canon = canonical_form(&r, &mut audit, "K-Refine V3 single");
        assert_eq!(canon, r, "single Refine must be a fixed point");
        assert!(audit.is_decidable());
    }

    #[test]
    fn check_round_trip_v2_admits_refine_fold_pair() {
        // Refine(Refine(B, x: p), x: q) is round-trip-equivalent to
        // Refine(B, x: p ∧ q) — V2 admits via the structural fold,
        // V0/V1 reject (not structurally identical).
        let b = var("Int");
        let inner = CoreTerm::Refine {
            base: Heap::new(b.clone()),
            binder: Text::from("x"),
            predicate: Heap::new(var("p")),
        };
        let nested = CoreTerm::Refine {
            base: Heap::new(inner),
            binder: Text::from("x"),
            predicate: Heap::new(var("q")),
        };
        let folded = CoreTerm::Refine {
            base: Heap::new(b),
            binder: Text::from("x"),
            predicate: Heap::new(crate::support::make_conjunction(&var("p"), &var("q"))),
        };
        let audit = check_round_trip_v2(&nested, &folded, "Refine fold").unwrap();
        assert!(audit.is_decidable(),
            "K-Refine V3 fold must be fully decidable — no Diakrisis bridge");
    }
}
