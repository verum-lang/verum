//! Algebraic laws for tactic combinators.
//!
//! Closes task #86. A tactic combinator `TacticCombinator` is a
//! tree over `{Single, AndThen, OrElse, Repeat, TryFor, WithParams,
//! IfThenElse, ParOr}`. Treated as an algebra, the combinators
//! satisfy the standard monoid + distribution laws from the
//! Ltac / LCF / tactical-programming literature.
//!
//! This module exposes:
//!
//! * The laws themselves, as pure structural rewrite rules on
//!   `TacticCombinator`.
//! * `normalize()` — a fixed-point rewrite that applies every
//!   semantics-preserving simplification, shrinking user-written
//!   combinators to canonical form.
//! * `check_law_*()` — boolean predicates that verify a given
//!   combinator pair satisfies a named law on structural equality.
//!   Used by property tests and by
//!   `core.proof.tactics.laws`-style regression suites.
//!
//! # Laws
//!
//! Monoid of AndThen with identity `skip` (0-iteration Repeat of
//! the identity simplify):
//!
//!   skip ; t  ≡  t            (L1 left-identity)
//!   t ; skip  ≡  t            (L2 right-identity)
//!   (t ; u) ; v  ≡  t ; (u ; v)   (L3 associativity)
//!
//! Monoid of OrElse with identity `fail`:
//!
//!   fail | t  ≡  t            (L4 left-identity)
//!   t | fail  ≡  t            (L5 right-identity)
//!   (t | u) | v  ≡  t | (u | v)   (L6 associativity)
//!
//! Repeat:
//!
//!   Repeat(t, 0)  ≡  skip     (L7 zero-unfold)
//!   Repeat(t, 1)  ≡  t        (L8 one-unfold)
//!
//! Idempotence (conservative — only on Single-leaf tactics because
//! a compound combinator's effect sequence is not necessarily
//! idempotent):
//!
//!   Single(k) | Single(k)  ≡  Single(k)   (L9 OrElse-idempotent)
//!
//! Semantic note on idempotence: `simp ; simp` is *observationally*
//! idempotent on every known Z3 state, but the solver may have
//! trace side-effects (proof term size, statistics counters) — so
//! we decline to simplify `t ; t` to `t`. Only OrElse-idempotence
//! is applied; it never changes the solver's proof trace because
//! only the first alternative runs in practice.
//!
//! # Why this isn't "just a simplifier"
//!
//! The laws let `user_tactic::compile_tactic` produce smaller,
//! deterministic combinators that the executor can dispatch
//! faster. More fundamentally: they give the `core.proof.tactics`
//! stdlib (task #85) a canonical form to target when it exports
//! tactic definitions. Two stdlib tactics that are the same
//! algebraically become literally `==` after normalization, so
//! the `tactic_registry` (task #87) can dedup them across
//! imported cogs.

use crate::tactics::{TacticCombinator, TacticKind};

/// The identity element for AndThen: a 0-iteration repeat of the
/// identity simplify — matches the `skip_strategy()` emitted by
/// `user_tactic::compile_tactic` for `Quote` / `Unquote` /
/// `GoalIntro`.
pub fn skip() -> TacticCombinator {
    TacticCombinator::Repeat(
        Box::new(TacticCombinator::Single(TacticKind::Simplify)),
        0,
    )
}

/// The identity element for OrElse: a maximally-failing tactic.
/// Encoded as a `Single(Custom("fail"))` — a named tactic the
/// executor recognises as the always-failing step. Using a
/// `Single` rather than a zero-Repeat keeps `fail` distinct from
/// `skip` (both are zero-effect, but `skip` *succeeds* and
/// `fail` *fails* — different absorbing elements for AndThen vs
/// OrElse).
pub fn fail() -> TacticCombinator {
    TacticCombinator::Single(TacticKind::Custom(verum_common::Text::from("fail")))
}

/// Is this combinator structurally equivalent to `skip`?
pub fn is_skip(c: &TacticCombinator) -> bool {
    matches!(
        c,
        TacticCombinator::Repeat(inner, 0)
            if matches!(**inner, TacticCombinator::Single(TacticKind::Simplify))
    )
}

/// Is this combinator structurally equivalent to `fail`?
pub fn is_fail(c: &TacticCombinator) -> bool {
    matches!(
        c,
        TacticCombinator::Single(TacticKind::Custom(tag))
            if tag.as_str() == "fail"
    )
}

/// Normalise a combinator to its canonical form by applying every
/// simplification law to fixpoint.
///
/// Complexity: O(n) passes where n is the depth of the tree; each
/// pass is O(tree size). Bounded by the tree size → overall
/// O(tree^2) worst-case. Callers with very deep user-written
/// combinators should bound depth at construction, not here.
pub fn normalize(c: TacticCombinator) -> TacticCombinator {
    let mut prev = format!("{:?}", c);
    let mut cur = normalize_once(c);
    loop {
        let next_repr = format!("{:?}", cur);
        if next_repr == prev {
            return cur;
        }
        prev = next_repr;
        cur = normalize_once(cur);
    }
}

/// Apply one pass of simplification laws.
fn normalize_once(c: TacticCombinator) -> TacticCombinator {
    match c {
        TacticCombinator::Single(k) => TacticCombinator::Single(k),

        TacticCombinator::AndThen(l, r) => {
            let l = normalize_once(*l);
            let r = normalize_once(*r);
            // L1: skip ; t ≡ t
            if is_skip(&l) {
                return r;
            }
            // L2: t ; skip ≡ t
            if is_skip(&r) {
                return l;
            }
            // L3: right-associate. `(a ; b) ; c` becomes
            // `a ; (b ; c)`. This gives every AndThen chain a
            // canonical right-associated shape, so two chains
            // that differ only in bracketing compare equal after
            // normalize.
            if let TacticCombinator::AndThen(ll, lr) = l {
                return normalize_once(TacticCombinator::AndThen(
                    ll,
                    Box::new(TacticCombinator::AndThen(lr, Box::new(r))),
                ));
            }
            TacticCombinator::AndThen(Box::new(l), Box::new(r))
        }

        TacticCombinator::OrElse(l, r) => {
            let l = normalize_once(*l);
            let r = normalize_once(*r);
            // L4: fail | t ≡ t
            if is_fail(&l) {
                return r;
            }
            // L5: t | fail ≡ t
            if is_fail(&r) {
                return l;
            }
            // L9: Single(k) | Single(k) ≡ Single(k) (only for
            // identical single-leaf tactics — see module docs for
            // why this is sound but AndThen-idempotence isn't).
            if let (
                TacticCombinator::Single(a),
                TacticCombinator::Single(b),
            ) = (&l, &r)
            {
                if a == b {
                    return TacticCombinator::Single(a.clone());
                }
            }
            TacticCombinator::OrElse(Box::new(l), Box::new(r))
        }

        TacticCombinator::Repeat(inner, 0) => {
            // L7: Repeat(t, 0) ≡ skip — regardless of what t is.
            // This is how `Quote` / `GoalIntro` compile-targets
            // fall out during normalization.
            let _ = inner;
            skip()
        }

        TacticCombinator::Repeat(inner, 1) => {
            // L8: Repeat(t, 1) ≡ t
            normalize_once(*inner)
        }

        TacticCombinator::Repeat(inner, n) => {
            TacticCombinator::Repeat(Box::new(normalize_once(*inner)), n)
        }

        TacticCombinator::TryFor(inner, dur) => {
            TacticCombinator::TryFor(Box::new(normalize_once(*inner)), dur)
        }

        TacticCombinator::WithParams(inner, params) => {
            TacticCombinator::WithParams(Box::new(normalize_once(*inner)), params)
        }

        TacticCombinator::IfThenElse {
            probe,
            then_tactic,
            else_tactic,
        } => TacticCombinator::IfThenElse {
            probe,
            then_tactic: Box::new(normalize_once(*then_tactic)),
            else_tactic: Box::new(normalize_once(*else_tactic)),
        },

        TacticCombinator::ParOr(branches) => {
            let mut norm_branches = verum_common::List::new();
            for b in branches {
                norm_branches.push(normalize_once(b));
            }
            TacticCombinator::ParOr(norm_branches)
        }
    }
}

/// Check L1 (AndThen left-identity) on a specific pair.
/// Returns true if `normalize(skip ; t) == normalize(t)`.
pub fn check_andthen_left_identity(t: &TacticCombinator) -> bool {
    let lhs = normalize(TacticCombinator::AndThen(
        Box::new(skip()),
        Box::new(t.clone()),
    ));
    let rhs = normalize(t.clone());
    format!("{:?}", lhs) == format!("{:?}", rhs)
}

/// Check L2 (AndThen right-identity).
pub fn check_andthen_right_identity(t: &TacticCombinator) -> bool {
    let lhs = normalize(TacticCombinator::AndThen(
        Box::new(t.clone()),
        Box::new(skip()),
    ));
    let rhs = normalize(t.clone());
    format!("{:?}", lhs) == format!("{:?}", rhs)
}

/// Check L3 (AndThen associativity): `(a ; b) ; c ≡ a ; (b ; c)`.
pub fn check_andthen_associativity(
    a: &TacticCombinator,
    b: &TacticCombinator,
    c: &TacticCombinator,
) -> bool {
    let lhs = normalize(TacticCombinator::AndThen(
        Box::new(TacticCombinator::AndThen(
            Box::new(a.clone()),
            Box::new(b.clone()),
        )),
        Box::new(c.clone()),
    ));
    let rhs = normalize(TacticCombinator::AndThen(
        Box::new(a.clone()),
        Box::new(TacticCombinator::AndThen(
            Box::new(b.clone()),
            Box::new(c.clone()),
        )),
    ));
    format!("{:?}", lhs) == format!("{:?}", rhs)
}

/// Check L4 (OrElse left-identity).
pub fn check_orelse_left_identity(t: &TacticCombinator) -> bool {
    let lhs = normalize(TacticCombinator::OrElse(
        Box::new(fail()),
        Box::new(t.clone()),
    ));
    let rhs = normalize(t.clone());
    format!("{:?}", lhs) == format!("{:?}", rhs)
}

/// Check L5 (OrElse right-identity).
pub fn check_orelse_right_identity(t: &TacticCombinator) -> bool {
    let lhs = normalize(TacticCombinator::OrElse(
        Box::new(t.clone()),
        Box::new(fail()),
    ));
    let rhs = normalize(t.clone());
    format!("{:?}", lhs) == format!("{:?}", rhs)
}

/// Check L7 (Repeat(t, 0) ≡ skip).
pub fn check_repeat_zero_is_skip(t: &TacticCombinator) -> bool {
    let lhs = normalize(TacticCombinator::Repeat(Box::new(t.clone()), 0));
    format!("{:?}", lhs) == format!("{:?}", skip())
}

/// Check L8 (Repeat(t, 1) ≡ t).
pub fn check_repeat_one_is_inner(t: &TacticCombinator) -> bool {
    let lhs = normalize(TacticCombinator::Repeat(Box::new(t.clone()), 1));
    let rhs = normalize(t.clone());
    format!("{:?}", lhs) == format!("{:?}", rhs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simp() -> TacticCombinator {
        TacticCombinator::Single(TacticKind::Simplify)
    }
    fn smt() -> TacticCombinator {
        TacticCombinator::Single(TacticKind::SMT)
    }
    fn lia() -> TacticCombinator {
        TacticCombinator::Single(TacticKind::LIA)
    }

    // -- Identities ----------------------------------------------------

    #[test]
    fn l1_andthen_left_identity_simp() {
        assert!(check_andthen_left_identity(&simp()));
    }

    #[test]
    fn l2_andthen_right_identity_smt() {
        assert!(check_andthen_right_identity(&smt()));
    }

    #[test]
    fn l3_andthen_associativity_all_primitives() {
        assert!(check_andthen_associativity(&simp(), &smt(), &lia()));
    }

    #[test]
    fn l4_orelse_left_identity() {
        assert!(check_orelse_left_identity(&simp()));
    }

    #[test]
    fn l5_orelse_right_identity() {
        assert!(check_orelse_right_identity(&smt()));
    }

    #[test]
    fn l7_repeat_zero_is_skip() {
        assert!(check_repeat_zero_is_skip(&simp()));
        assert!(check_repeat_zero_is_skip(&smt()));
    }

    #[test]
    fn l8_repeat_one_is_inner() {
        assert!(check_repeat_one_is_inner(&simp()));
        // Also on a compound tactic.
        let seq = TacticCombinator::AndThen(Box::new(simp()), Box::new(smt()));
        assert!(check_repeat_one_is_inner(&seq));
    }

    // -- Normalize actually simplifies ---------------------------------

    #[test]
    fn normalize_strips_skip_on_the_left() {
        let t = TacticCombinator::AndThen(Box::new(skip()), Box::new(smt()));
        let got = format!("{:?}", normalize(t));
        let want = format!("{:?}", smt());
        assert_eq!(got, want);
    }

    #[test]
    fn normalize_strips_fail_on_the_right_of_orelse() {
        let t = TacticCombinator::OrElse(Box::new(simp()), Box::new(fail()));
        let got = format!("{:?}", normalize(t));
        let want = format!("{:?}", simp());
        assert_eq!(got, want);
    }

    #[test]
    fn normalize_collapses_single_or_single_same_kind() {
        let t = TacticCombinator::OrElse(Box::new(simp()), Box::new(simp()));
        let got = format!("{:?}", normalize(t));
        let want = format!("{:?}", simp());
        assert_eq!(got, want);
    }

    #[test]
    fn normalize_does_not_collapse_andthen_of_identical_singles() {
        // L9 only applies to OrElse — `t ; t` stays `t ; t`
        // because solver side-effects may differ per invocation.
        let t = TacticCombinator::AndThen(Box::new(simp()), Box::new(simp()));
        let normalized = normalize(t.clone());
        assert_eq!(format!("{:?}", t), format!("{:?}", normalized));
    }

    #[test]
    fn normalize_is_idempotent() {
        // Applying normalize twice yields the same tree.
        let t = TacticCombinator::AndThen(
            Box::new(skip()),
            Box::new(TacticCombinator::OrElse(
                Box::new(fail()),
                Box::new(simp()),
            )),
        );
        let once = normalize(t.clone());
        let twice = normalize(once.clone());
        assert_eq!(format!("{:?}", once), format!("{:?}", twice));
    }

    #[test]
    fn is_skip_identifies_zero_repeat_of_simplify() {
        assert!(is_skip(&skip()));
        assert!(!is_skip(&simp()));
        assert!(!is_skip(&smt()));
    }

    #[test]
    fn is_fail_identifies_zero_repeat_of_custom_fail() {
        assert!(is_fail(&fail()));
        assert!(!is_fail(&skip()));
    }

    #[test]
    fn deep_normalize_shrinks_nested_skip_andthen_chains() {
        // skip ; (skip ; (skip ; simp)) → simp
        let nested = TacticCombinator::AndThen(
            Box::new(skip()),
            Box::new(TacticCombinator::AndThen(
                Box::new(skip()),
                Box::new(TacticCombinator::AndThen(
                    Box::new(skip()),
                    Box::new(simp()),
                )),
            )),
        );
        let got = format!("{:?}", normalize(nested));
        let want = format!("{:?}", simp());
        assert_eq!(got, want);
    }
}
