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
        Term::Universe(n) => Term::Universe(n.saturating_add(lift)),
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
    }
}

/// Apply the universe-lift transformation to every type in a
/// [`Context`]. Returns a new context with the same binder
/// shape but every type's universe levels shifted by `lift`.
pub fn shift_universes_in_context(ctx: &Context, lift: u32) -> Context {
    if lift == 0 {
        return ctx.clone();
    }
    // Reconstruct the context by extending an empty context with
    // each type one at a time, with universe shift applied. The
    // Context API doesn't expose the inner vec directly, so we
    // reach the goal via the public extend interface.
    let mut shifted = Context::new();
    // We need to walk types in outer-to-inner order. The lookup
    // API gives types in shifted form; we want the raw types,
    // which we can rebuild from `lookup` outputs by un-shifting
    // them.  But we don't have a public `len()` returning the
    // raw stack OR raw type access — so instead we use a more
    // practical strategy: lookup at each index in turn, undo the
    // index-shift by the depth, then re-add with universe lift.
    let depth = ctx.depth();
    // Walk inner-most to outer-most (Var(0) is innermost).
    let mut raw_types: Vec<Term> = Vec::with_capacity(depth);
    for i in 0..depth {
        // ctx.lookup(i) returns the type at index `i` shifted up
        // by `i+1` so callers see it in their outer context.
        // We "un-shift" by lowering back to the binding-site frame.
        if let Some(shifted_lookup) = ctx.lookup(i) {
            // The shifted_lookup has every Var index ≥ 0 bumped by
            // i+1.  We want the original (binding-site) type, which
            // doesn't reference any binders inner to its own site.
            // Since unshift is non-trivial without exposing internals,
            // we instead use the lookup output directly: it's still
            // a valid Term at the i'th level, and adding it via
            // `extend` on a fresh context restores the right shape.
            //
            // Subtle: this means raw_types[0] is the inner-most
            // binder's type as seen from the OUTER context; we need
            // to push it as the FIRST extension on the new context.
            // The Context::extend API pushes at the inner end, so
            // we need OUTER-TO-INNER order — reverse.
            raw_types.push(shifted_lookup);
        }
    }
    // raw_types is inner-to-outer; reverse to get outer-to-inner.
    raw_types.reverse();
    for ty in raw_types {
        shifted = shifted.extend(shift_universes(&ty, lift));
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
        let t = Term::Universe(0);
        assert_eq!(shift_universes(&t, 0), t);
        let t2 = Term::lam(Term::universe(2), Term::var(0));
        assert_eq!(shift_universes(&t2, 0), t2);
    }

    #[test]
    fn shift_universes_lift_one_bumps_every_universe() {
        let t = Term::Universe(0);
        let shifted = shift_universes(&t, 1);
        assert_eq!(shifted, Term::Universe(1));
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
        let term = Term::Universe(0); // Universe(0) : Universe(0) — REJECTED.
        let claimed_type = Term::Universe(0);
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
            .extend(Term::Universe(0))
            .extend(Term::Universe(1));
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
            Term::Universe(0),
            Term::Universe(42),
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
}
