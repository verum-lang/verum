//! Meta-universe lift mechanism for the proof-term checker
//! (#158 V1 — kernel reflection / Gödel-2nd workaround foundation).
//!
//! ## Architectural role
//!
//! Verum's kernel-soundness claim is necessarily proven in a
//! slightly stronger meta-theory than the kernel itself, per
//! Gödel's Second Incompleteness Theorem. The structured escape
//! is to run the kernel with one extra strongly-inaccessible
//! cardinal `κ_meta` above the working universe.
//!
//! This module ships the **algorithmic foundation** for that
//! escape: a [`shift_universes`] helper that walks a `Term` and
//! shifts every `Universe(n)` by a fixed offset, and the
//! [`check_with_universe_lift`] wrapper that runs the existing
//! [`crate::proof_checker::check`] with the shifted term + type +
//! context. Universe-stability under lift becomes the
//! algorithmic invariant pinning meta-soundness.
//!
//! ## What "running in meta-mode" means
//!
//! At `lift = 0` (default), the checker runs at its native universe
//! hierarchy — every `Universe(n)` means level `n`. At `lift = k`,
//! every `Universe(n)` is interpreted as `Universe(n + k)` —
//! semantically: the entire term lives in the universe `k` levels
//! above the original. Closed certificates that type-check at
//! `lift = 0` MUST also type-check at any `lift = k > 0`, because
//! the universe hierarchy is monotone (HTT 1.4: `U_n ⊂ U_{n+1}`).
//!
//! Disagreement between `lift = 0` and `lift = k` would indicate
//! either:
//!   * A bug in the universe-shift implementation.
//!   * A bug in the underlying type checker that's universe-
//!     hierarchy-dependent in a way it shouldn't be.
//!
//! Either way, the audit gate catches it.
//!
//! ## Soundness invariant (the load-bearing claim)
//!
//!   For every closed certificate `cert` and lift `k ≥ 0`:
//!     `check(cert) accepts ⟺ check_with_lift(cert, k) accepts`
//!
//! This is the **universe-stability invariant** — kernel
//! verdicts must be invariant under monotone universe shift.
//!
//! ## Use cases
//!
//!   * `verum check --meta-mode` — run any proof at lift = 1,
//!     verifying it survives the meta-level interpretation.
//!   * `verum audit --meta-universe-stability` — walk the
//!     canonical certificate library at lifts 0/1/2/3 and check
//!     every certificate's verdict is stable.
//!   * Differential testing across universe levels: a regression
//!     where a proof passes at level 0 but fails at level 1
//!     surfaces an unsound dependence on universe identity.

use crate::proof_checker::{Certificate, CheckError, Context, Term, check};

// =============================================================================
// Universe-shift transformation
// =============================================================================

/// Recursively walk a `Term` and add `lift` to every
/// `Universe(n)` it contains. Variable indices and term
/// structure are preserved; only universe levels shift.
///
/// **Soundness**: monotone — every `Universe(n)` is replaced by
/// `Universe(n + lift)`. The universe hierarchy is cumulative
/// (HTT 1.4: `Universe(n) : Universe(n+1)`), so lifting preserves
/// type-checking; a term valid at level 0 is valid at any
/// higher level.
///
/// **Performance**: O(n) recursive walk. Pure transformation, no
/// allocation beyond the rebuilt Term tree.
pub fn shift_universes(term: &Term, lift: u32) -> Term {
    if lift == 0 {
        return term.clone();
    }
    match term {
        Term::Var(i) => Term::Var(*i),
        Term::Universe(level) => Term::Universe(level.clone().shifted_by(lift)),
        Term::Pi(a, b) => Term::Pi(
            Box::new(shift_universes(a, lift)),
            Box::new(shift_universes(b, lift)),
        ),
        Term::Lam(a, body) => Term::Lam(
            Box::new(shift_universes(a, lift)),
            Box::new(shift_universes(body, lift)),
        ),
        Term::App(f, x) => Term::App(
            Box::new(shift_universes(f, lift)),
            Box::new(shift_universes(x, lift)),
        ),
        Term::Sigma(a, b) => Term::Sigma(
            Box::new(shift_universes(a, lift)),
            Box::new(shift_universes(b, lift)),
        ),
        Term::Pair(a, b) => Term::Pair(
            Box::new(shift_universes(a, lift)),
            Box::new(shift_universes(b, lift)),
        ),
        Term::Fst(p) => Term::Fst(Box::new(shift_universes(p, lift))),
        Term::Snd(p) => Term::Snd(Box::new(shift_universes(p, lift))),
        Term::Id { ty, lhs, rhs } => Term::Id {
            ty: Box::new(shift_universes(ty, lift)),
            lhs: Box::new(shift_universes(lhs, lift)),
            rhs: Box::new(shift_universes(rhs, lift)),
        },
        Term::Refl(value) => Term::Refl(Box::new(shift_universes(value, lift))),
        Term::J {
            motive,
            base,
            scrutinee,
        } => Term::J {
            motive: Box::new(shift_universes(motive, lift)),
            base: Box::new(shift_universes(base, lift)),
            scrutinee: Box::new(shift_universes(scrutinee, lift)),
        },
    }
}

/// Apply the universe-lift transformation to every type in a
/// [`Context`]. Returns a new context with the same binder
/// shape but every type's universe levels shifted by `lift`.
///
/// **Soundness**: walks raw binding-site types via
/// [`Context::iter_outer_to_inner`] and applies [`shift_universes`]
/// at each binder's own frame.  Inter-binder references (e.g. `B`
/// having type `A → Type` in `[A : Type, B : A → Type]`) survive
/// exactly: the rebuilt context preserves every de Bruijn index
/// at its binding-site meaning.
///
/// The earlier implementation used `Context::lookup` (which
/// **shifts the result up to the outer frame**) and pushed the
/// shifted Term back without unshifting — silently corrupting any
/// context whose types referenced outer binders.  The fix routes
/// through the new raw-types API.
pub fn shift_universes_in_context(ctx: &Context, lift: u32) -> Context {
    if lift == 0 {
        return ctx.clone();
    }
    let mut shifted = Context::new();
    for raw_ty in ctx.iter_outer_to_inner() {
        shifted = shifted.extend(shift_universes(raw_ty, lift));
    }
    shifted
}

// =============================================================================
// check_with_universe_lift — the meta-mode entry point
// =============================================================================

/// Type-check `term` against `expected` under `ctx`, with every
/// universe level in all three shifted up by `lift`. Equivalent
/// to running the checker with the entire problem moved up
/// `lift` levels in the universe hierarchy.
///
/// **Lift = 0** is the default — identical to
/// [`crate::proof_checker::check`].
///
/// **Lift > 0** is the meta-mode case — the kernel verifies the
/// proof at a strictly stronger universe. Used by:
///   * `verum check --meta-mode` (lift = 1) to confirm a proof
///     survives meta-level interpretation.
///   * `verum audit --meta-universe-stability` to walk the
///     canonical certificate library and pin universe-stability
///     across multiple lifts.
///
/// **Soundness invariant**: for every closed certificate `cert`
/// and lift `k ≥ 0`,
///   `check(cert) accepts ⟺ check_with_universe_lift(cert, k) accepts`.
/// Disagreements indicate either a shift-implementation bug or a
/// kernel bug with unsound universe-identity dependence.
pub fn check_with_universe_lift(
    ctx: &Context,
    term: &Term,
    expected: &Term,
    lift: u32,
) -> Result<(), CheckError> {
    let shifted_ctx = shift_universes_in_context(ctx, lift);
    let shifted_term = shift_universes(term, lift);
    let shifted_expected = shift_universes(expected, lift);
    check(&shifted_ctx, &shifted_term, &shifted_expected)
}

/// Verify a [`Certificate`] under a universe lift. Architectural
/// twin of [`Certificate::verify`] for meta-mode.
pub fn verify_certificate_with_lift(
    cert: &Certificate,
    lift: u32,
) -> Result<(), CheckError> {
    let ctx = Context::new();
    check_with_universe_lift(&ctx, &cert.term, &cert.claimed_type, lift)
}

/// **The universe-stability check** — verify that a certificate's
/// verdict is stable across a range of lifts. Returns
/// `(verdicts_at_each_lift, all_agree)` where `verdicts_at_each_lift`
/// is `Vec<Result<(), CheckError>>` for lifts `0..=max_lift` and
/// `all_agree` is `true` iff every lift produces the SAME accept/
/// reject classification (the actual error reasons may differ but
/// the boolean accept/reject must be uniform).
///
/// **Architectural use**: the audit gate
/// `verum audit --meta-universe-stability` calls this for every
/// certificate in `core/verify/proof_term_examples/` plus the
/// adversarial library; any non-stable verdict flips the gate to
/// failure.
pub fn check_universe_stability(
    cert: &Certificate,
    max_lift: u32,
) -> (Vec<Result<(), CheckError>>, bool) {
    let mut verdicts = Vec::with_capacity((max_lift + 1) as usize);
    for lift in 0..=max_lift {
        verdicts.push(verify_certificate_with_lift(cert, lift));
    }
    let all_accept = verdicts.iter().all(|v| v.is_ok());
    let all_reject = verdicts.iter().all(|v| v.is_err());
    let stable = all_accept || all_reject;
    (verdicts, stable)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn polymorphic_identity() -> Certificate {
        let term = Term::lam(
            Term::universe(0),
            Term::lam(Term::var(0), Term::var(0)),
        );
        let claimed_type = Term::pi(
            Term::universe(0),
            Term::pi(Term::var(0), Term::var(1)),
        );
        Certificate {
            term,
            claimed_type,
            metadata: std::collections::BTreeMap::new(),
        }
    }

    fn identity_universe_0() -> Certificate {
        let term = Term::lam(Term::universe(0), Term::var(0));
        let claimed_type = Term::pi(Term::universe(0), Term::universe(0));
        Certificate {
            term,
            claimed_type,
            metadata: std::collections::BTreeMap::new(),
        }
    }

    // ----- shift_universes -----

    #[test]
    fn shift_universes_zero_lift_is_identity() {
        let t = Term::universe(0);
        assert_eq!(shift_universes(&t, 0), t);
        let t2 = Term::lam(Term::universe(2), Term::var(0));
        assert_eq!(shift_universes(&t2, 0), t2);
    }

    #[test]
    fn shift_universes_lift_one_bumps_every_universe() {
        let t = Term::universe(0);
        let shifted = shift_universes(&t, 1);
        assert_eq!(shifted, Term::universe(1));
    }

    #[test]
    fn shift_universes_recurses_through_pi() {
        let t = Term::pi(Term::universe(0), Term::universe(1));
        let shifted = shift_universes(&t, 2);
        assert_eq!(shifted, Term::pi(Term::universe(2), Term::universe(3)));
    }

    #[test]
    fn shift_universes_recurses_through_lam_and_app() {
        let t = Term::lam(
            Term::universe(0),
            Term::app(Term::universe(1), Term::var(0)),
        );
        let shifted = shift_universes(&t, 1);
        assert_eq!(
            shifted,
            Term::lam(
                Term::universe(1),
                Term::app(Term::universe(2), Term::var(0)),
            )
        );
    }

    #[test]
    fn shift_universes_preserves_var_indices() {
        // Var indices must NOT be touched by universe shift.
        let t = Term::lam(
            Term::universe(0),
            Term::lam(Term::universe(0), Term::var(1)),
        );
        let shifted = shift_universes(&t, 5);
        // Var(1) stays Var(1), only Universe(0) becomes Universe(5).
        match shifted {
            Term::Lam(_, inner) => match *inner {
                Term::Lam(_, body) => {
                    assert!(matches!(*body, Term::Var(1)));
                }
                other => panic!("expected nested Lam, got {:?}", other),
            },
            other => panic!("expected Lam, got {:?}", other),
        }
    }

    // ----- check_with_universe_lift -----

    #[test]
    fn check_with_lift_zero_matches_check_at_native() {
        let cert = polymorphic_identity();
        let native = Certificate::verify(&cert);
        let lifted = verify_certificate_with_lift(&cert, 0);
        assert_eq!(native.is_ok(), lifted.is_ok());
    }

    #[test]
    fn check_with_lift_one_accepts_polymorphic_identity() {
        let cert = polymorphic_identity();
        let result = verify_certificate_with_lift(&cert, 1);
        assert!(
            result.is_ok(),
            "polymorphic identity must accept under lift=1: {:?}",
            result,
        );
    }

    #[test]
    fn check_with_lift_higher_levels_also_accept() {
        let cert = polymorphic_identity();
        for lift in 0..=5 {
            let result = verify_certificate_with_lift(&cert, lift);
            assert!(
                result.is_ok(),
                "polymorphic identity must be stable under lift={}: {:?}",
                lift,
                result,
            );
        }
    }

    // ----- check_universe_stability (the load-bearing pin) -----

    #[test]
    fn polymorphic_identity_is_universe_stable() {
        let cert = polymorphic_identity();
        let (verdicts, stable) = check_universe_stability(&cert, 5);
        assert!(stable, "polymorphic identity must be universe-stable");
        for v in &verdicts {
            assert!(v.is_ok(), "every lift must accept: {:?}", v);
        }
    }

    #[test]
    fn identity_universe_0_is_universe_stable() {
        let cert = identity_universe_0();
        let (verdicts, stable) = check_universe_stability(&cert, 5);
        assert!(stable, "identity_universe_0 must be universe-stable");
        for v in &verdicts {
            assert!(v.is_ok(), "every lift must accept: {:?}", v);
        }
    }

    #[test]
    fn invalid_certificate_is_universe_stable_in_rejection() {
        // A certificate that's REJECTED at lift 0 must also be
        // rejected at every higher lift. Universe-stability cuts
        // both ways: true verdicts stay true, false verdicts stay
        // false.
        let term = Term::universe(0); // Universe(0) : Universe(0) — REJECTED.
        let claimed_type = Term::universe(0);
        let cert = Certificate {
            term,
            claimed_type,
            metadata: std::collections::BTreeMap::new(),
        };
        let (verdicts, stable) = check_universe_stability(&cert, 5);
        assert!(stable, "invalid cert must reject at every lift");
        for v in &verdicts {
            assert!(v.is_err(), "every lift must reject: {:?}", v);
        }
    }

    // ----- shift_universes_in_context -----

    #[test]
    fn shift_universes_in_empty_context_is_empty() {
        let ctx = Context::new();
        let shifted = shift_universes_in_context(&ctx, 5);
        assert_eq!(shifted.depth(), 0);
    }

    #[test]
    fn shift_universes_in_context_preserves_depth() {
        let ctx = Context::new()
            .extend(Term::universe(0))
            .extend(Term::universe(1));
        let shifted = shift_universes_in_context(&ctx, 1);
        assert_eq!(shifted.depth(), ctx.depth());
    }

    // ----- Architectural pin: lift=0 is the algorithmic identity -----

    #[test]
    fn shift_universes_zero_is_function_identity() {
        // Pin: shift_universes(t, 0) MUST be exactly t. Any
        // implementation drift here is a soundness regression.
        let probes = [
            Term::Var(0),
            Term::universe(0),
            Term::universe(42),
            Term::pi(Term::universe(1), Term::universe(2)),
            Term::lam(
                Term::universe(0),
                Term::app(Term::var(0), Term::var(0)),
            ),
        ];
        for p in &probes {
            assert_eq!(&shift_universes(p, 0), p);
        }
    }

    // ----- Soundness pins for the corrected
    //       `shift_universes_in_context` (the fix for the
    //       lookup-as-raw-type kludge that silently corrupted
    //       contexts whose types referenced outer binders).
    //
    // The invariant: for any context Γ, the shifted context Γ′ has
    // the SAME structural shape as Γ, with every Universe(level)
    // replaced by Universe(level + lift).  Inter-binder references
    // must survive verbatim — no de Bruijn index should change.

    #[test]
    fn shift_in_context_preserves_inter_binder_reference() {
        // Γ = [A : Type@0, B : A → Type@0]
        //   B's type is `Π(_:Var(0)). Type@0` at the binding site —
        //   Var(0) refers to the outer A.
        // Γ′ = shift_universes_in_context(Γ, 1) MUST give:
        //   [A : Type@1, B : Π(_:Var(0)). Type@1]
        //   The Var(0) in B's domain must STILL point to A.
        let inner_ty = Term::pi(Term::Var(0), Term::universe(0));
        let ctx = Context::new()
            .extend(Term::universe(0))
            .extend(inner_ty);
        let shifted = shift_universes_in_context(&ctx, 1);

        // Depth preserved.
        assert_eq!(shifted.depth(), 2);

        // Raw types: outer is Type@1, inner is Π(_:Var(0)). Type@1.
        let raw = shifted.types();
        assert_eq!(raw[0], Term::universe(1));
        assert_eq!(
            raw[1],
            Term::pi(Term::Var(0), Term::universe(1)),
            "inter-binder Var(0) reference must survive shift",
        );
    }

    #[test]
    fn shift_in_context_universe_stability_on_dependent_pi() {
        // Pin the load-bearing universe-stability invariant on a
        // genuinely dependent setting: a closed certificate using a
        // dependent Π over a hypothesis context with inter-binder
        // references must accept at every lift it accepts at lift 0.
        //
        // Setup: Γ = [A : Type@0]; check that
        //   `λ(x : Var(0)). x : Π(_:Var(0)). Var(1)`
        // type-checks under Γ at lift 0, then at lift 1, 2, 3 with
        // shift_universes_in_context applied — they must all agree.
        let ctx = Context::new().extend(Term::universe(0));
        let term = Term::lam(Term::Var(0), Term::Var(0));
        let claim = Term::pi(Term::Var(0), Term::Var(1));

        // Ground-truth: lift 0 accepts.
        let ground = check_with_universe_lift(&ctx, &term, &claim, 0);
        assert!(ground.is_ok(), "lift=0 should accept: {:?}", ground);

        // Universe-stability across non-trivial lifts.
        for lift in [1u32, 2, 3, 7] {
            let v = check_with_universe_lift(&ctx, &term, &claim, lift);
            assert!(
                v.is_ok(),
                "lift={lift} should agree with lift=0: {:?}",
                v,
            );
        }
    }

    #[test]
    fn shift_in_context_raw_types_outer_to_inner() {
        // The new `iter_outer_to_inner` API gives types in the
        // order they were extended — outermost first, innermost
        // last.  Pin the iteration order.
        let ctx = Context::new()
            .extend(Term::universe(0))
            .extend(Term::universe(1))
            .extend(Term::universe(2));
        let collected: Vec<&Term> = ctx.iter_outer_to_inner().collect();
        assert_eq!(collected.len(), 3);
        assert_eq!(*collected[0], Term::universe(0));
        assert_eq!(*collected[1], Term::universe(1));
        assert_eq!(*collected[2], Term::universe(2));
    }

    #[test]
    fn shift_in_context_idempotent_on_lift_zero_with_dependencies() {
        // For non-trivial contexts, lift=0 must still produce an
        // identical context (the early-return path).  Any drift
        // here would mean the rewrite changed semantics on the
        // baseline.
        let inner_ty = Term::pi(Term::Var(0), Term::universe(0));
        let ctx = Context::new()
            .extend(Term::universe(0))
            .extend(inner_ty);
        let shifted = shift_universes_in_context(&ctx, 0);
        assert_eq!(shifted.types(), ctx.types());
    }

    #[test]
    fn shift_in_context_three_deep_chain() {
        // Γ = [A : Type@0, B : A → Type@0, C : B(_) → Type@0]
        //   Each binder references its predecessor — every de
        //   Bruijn index must survive lift unchanged.
        // Body of C's domain references B as Var(1) (since A's
        // entry has shifted under B's binder); we model this
        // structurally: ctx[2] is `Π(_:App(Var(0), Var(1))). Type@0`.
        let outer_a = Term::universe(0);
        let mid_b = Term::pi(Term::Var(0), Term::universe(0));
        let inner_c = Term::pi(
            Term::app(Term::Var(0), Term::Var(1)),
            Term::universe(0),
        );
        let ctx = Context::new()
            .extend(outer_a)
            .extend(mid_b)
            .extend(inner_c);
        let shifted = shift_universes_in_context(&ctx, 4);
        let raw = shifted.types();
        assert_eq!(raw.len(), 3);
        assert_eq!(raw[0], Term::universe(4));
        assert_eq!(
            raw[1],
            Term::pi(Term::Var(0), Term::universe(4)),
            "B's reference to A must survive at index 0",
        );
        assert_eq!(
            raw[2],
            Term::pi(
                Term::app(Term::Var(0), Term::Var(1)),
                Term::universe(4),
            ),
            "C's references to B (Var(0)) and A (Var(1)) must both survive",
        );
    }
}
